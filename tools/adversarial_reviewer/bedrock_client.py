"""Bounded, tool-free Bedrock calls using short-lived AWS credentials."""
import json
import os
import re
import subprocess
import tempfile
import time

MAX_RESPONSE_BYTES = 1048576  # 1 MiB
MAX_REQUEST_BYTES = 524288  # 512 KiB
MODEL = 'us.anthropic.claude-haiku-4-5-20251001-v1:0'


class BedrockError(Exception):
    """Content-free diagnostic safe to publish in a review report."""


def complete(messages, deadline):
    if os.environ.get('BEDROCK_REVIEW_ENABLED') != 'true':
        raise BedrockError('bedrock review disabled')
    region = os.environ.get('AWS_REGION')
    model = os.environ.get('BEDROCK_REVIEW_MODEL')
    if region != 'us-east-1' or model != MODEL:
        raise BedrockError('bedrock region or model is not configured for this reviewer')
    remaining = min(240, deadline - time.monotonic())
    if remaining <= 0:
        raise BedrockError('bedrock assessment deadline exhausted')
    system = [{'text': m['content']} for m in messages if m['role'] == 'system']
    conversation = [{'role': m['role'], 'content': [{'text': m['content']}]} for m in messages if m['role'] != 'system']
    request = json.dumps({'modelId': model, 'system': system, 'messages': conversation,
                          'inferenceConfig': {'maxTokens': 8192, 'temperature': 0}})
    if len(request.encode()) > MAX_REQUEST_BYTES:
        raise BedrockError('bedrock request exceeds size limit')
    # A private temporary file keeps review evidence out of process arguments.
    env = {key: value for key, value in os.environ.items() if key.startswith('AWS_') or key in ('PATH', 'HOME')}
    env.update(AWS_MAX_ATTEMPTS='1', AWS_PAGER='')
    try:
        with tempfile.NamedTemporaryFile(mode='w+', suffix='.json') as payload:
            payload.write(request)
            payload.flush()
            result = subprocess.run(['aws', 'bedrock-runtime', 'converse', '--region', region,
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
        envelope = json.loads(result.stdout)
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
