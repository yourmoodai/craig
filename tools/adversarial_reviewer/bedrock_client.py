"""Bounded, tool-free Bedrock calls using short-lived AWS credentials."""
import hashlib
import json
import os
import re
import subprocess
import tempfile
import time

from budget import Ledger, BudgetError

MAX_RESPONSE_BYTES = 1048576  # 1 MiB
MAX_REQUEST_BYTES = 524288  # 512 KiB

MODEL = 'us.anthropic.claude-haiku-4-5-20251001-v1:0'


class BedrockError(Exception):
    """Content-free diagnostic safe to publish in a review report."""


def conversation(messages):
    return {'system': [{'text': m['content']} for m in messages if m['role'] == 'system'],
            'messages': [{'role': m['role'], 'content': [{'text': m['content']}]} for m in messages if m['role'] != 'system']}


def count_tokens(messages, deadline=None):
    if os.environ.get('AWS_REGION') != 'us-east-1':
        raise BedrockError('token counting requires explicit us-east-1 region')
    envelope = invoke('count-tokens', {'modelId': MODEL.removeprefix('us.'),
                      'input': {'converse': conversation(messages)}}, deadline or time.monotonic()+240)
    value = envelope.get('inputTokens')
    if type(value) is not int or value < 0:
        raise BedrockError('invalid token count')
    return value


def invoke(operation, payload, deadline):
    remaining = min(240, deadline - time.monotonic())
    if remaining <= 0:
        raise BedrockError('bedrock assessment deadline exhausted')
    region = 'us-east-1'
    request = json.dumps(payload)
    if len(request.encode()) > MAX_REQUEST_BYTES:
        raise BedrockError('bedrock request exceeds size limit')
    # A private temporary file keeps review evidence out of process arguments.
    env = {key: value for key, value in os.environ.items() if key.startswith('AWS_') or key in ('PATH', 'HOME')}
    env.update(AWS_MAX_ATTEMPTS='1', AWS_PAGER='')
    try:
        with tempfile.NamedTemporaryFile(mode='w+', suffix='.json') as payload:
            payload.write(request)
            payload.flush()
            result = subprocess.run(['aws', 'bedrock-runtime', operation, '--region', region,
                                     '--cli-input-json', 'file://'+payload.name, '--output', 'json',
                                     '--cli-connect-timeout', '10', '--cli-read-timeout', str(max(1, int(remaining)))],
                                    text=True, capture_output=True, env=env, timeout=remaining, check=False)
    except subprocess.TimeoutExpired:
        raise BedrockError('bedrock call exceeded its execution deadline') from None
    except OSError:
        raise BedrockError('aws cli unavailable') from None
    if result.returncode:
        raise BedrockError('bedrock invocation failed; check role permissions, model access and quota')
    if len(result.stdout.encode()) > MAX_RESPONSE_BYTES:
        raise BedrockError('bedrock response exceeds size limit')
    try:
        value = json.loads(result.stdout)
        if not isinstance(value, dict):
            raise ValueError()
        return value
    except (ValueError, TypeError):
        raise BedrockError('bedrock returned invalid JSON') from None


def complete(messages, deadline, *, purpose='assessment', identity=None):
    if os.environ.get('BEDROCK_REVIEW_ENABLED') != 'true':
        raise BedrockError('bedrock review disabled')
    if os.environ.get('AWS_REGION') != 'us-east-1' or os.environ.get('BEDROCK_REVIEW_MODEL') != MODEL:
        raise BedrockError('bedrock region or model is not configured for this reviewer')
    payload = {'modelId': MODEL, **conversation(messages),
               'inferenceConfig': {'maxTokens': 512 if purpose == 'overview' else 4096, 'temperature': 0}}
    if len(json.dumps(payload).encode()) > MAX_REQUEST_BYTES:
        raise BedrockError('bedrock request exceeds size limit')
    if deadline <= time.monotonic():
        raise BedrockError('bedrock assessment deadline exhausted')
    path = os.environ.get('REVIEW_BUDGET_LEDGER')
    if not path:
        raise BedrockError('persistent review budget ledger required')
    inputs = count_tokens(messages, deadline)
    if inputs > 48000:
        raise BedrockError('prompt exceeds token budget; subdivision required')
    ledger = Ledger(path)
    identity = identity or hashlib.sha256(json.dumps(messages, sort_keys=True).encode()).hexdigest()
    try:
        key = ledger.reserve(inputs, payload['inferenceConfig']['maxTokens'],
                             os.environ.get('REVIEW_BUDGET_PHASE', 'review'), purpose+':'+identity)
        started = time.monotonic()
        envelope = invoke('converse', payload, deadline)
        ledger.reconcile(key, envelope, time.monotonic()-started)
    except BudgetError as error:
        raise BedrockError(str(error)) from None
    try:
        if envelope.get('stopReason') == 'max_tokens':
            raise BedrockError('response incomplete: subdivision required')
        if envelope.get('stopReason') != 'end_turn':
            raise BedrockError('bedrock response incomplete or blocked')
        content = envelope['output']['message']['content']
        if not content or any(set(block) != {'text'} for block in content):
            raise ValueError()
        raw = ''.join(block['text'] for block in content).strip()
        fenced = re.fullmatch(r'```(?:json)?\s*\n(.*?)\n```', raw, re.DOTALL)
        value = json.loads(fenced.group(1) if fenced else raw)
        if not isinstance(value, dict):
            raise ValueError()
        return value
    except (ValueError, KeyError, TypeError, AttributeError):
        raise BedrockError('bedrock returned invalid JSON') from None
