import json
import os
import subprocess
import time
import tempfile
import unittest
from unittest.mock import patch
import bedrock_client as client


class BedrockClientTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        env = patch.dict(os.environ, BEDROCK_REVIEW_ENABLED='true', AWS_REGION='us-east-1',
                         BEDROCK_REVIEW_MODEL=client.MODEL, GITHUB_TOKEN='private canary', REVIEW_BUDGET_LEDGER=temp.name+'/ledger.db')
        env.start()
        self.addCleanup(env.stop)

    def invoke(self, text='{}', stop='end_turn', code=0):
        output = json.dumps({'usage': {'inputTokens':20, 'outputTokens':20}, 'stopReason': stop, 'output': {'message': {'content': [{'text': text}]}}})
        def respond(argv, **kwargs):
            from pathlib import Path
            self.request = json.loads(Path(argv[argv.index('--cli-input-json')+1].removeprefix('file://')).read_text())
            return subprocess.CompletedProcess([], code, output, 'private canary')
        with patch('bedrock_client.subprocess.run', side_effect=respond) as run:
            value = client.complete([{'role': 'system', 'content': 'rules'}, {'role': 'user', 'content': 'synthetic diff'}], time.monotonic()+10)
            self.assertEqual(run.call_count, 1)
            self.assertIn('converse', run.call_args.args[0])
            return value, run.call_args

    def test_bounded_tool_free_request(self):
        value, call = self.invoke('```json\n{"findings":[]}\n```')
        self.assertEqual(value, {'findings': []})
        request = self.request
        self.assertEqual(request['system'], [{'text': 'rules'}])
        self.assertEqual(request['inferenceConfig']['maxTokens'], 1024)
        self.assertNotIn('toolConfig', request)
        self.assertNotIn('GITHUB_TOKEN', call.kwargs['env'])
        self.assertEqual(call.kwargs['env']['AWS_MAX_ATTEMPTS'], '1')
        self.assertLessEqual(call.kwargs['timeout'], 10)

    def test_truncated_or_blocked_results_never_approve(self):
        for reason in ['max_tokens', 'tool_use', 'content_filtered', None]:
            with self.assertRaisesRegex(client.BedrockError, 'incomplete'):
                self.invoke('{}', stop=reason)

    def test_invalid_outputs_and_provider_errors_are_sanitized(self):
        for text in ['[]', '{', 'private canary', 'x'*client.MAX_RESPONSE_BYTES]:
            with self.assertRaises(client.BedrockError) as error:
                self.invoke(text)
            self.assertNotIn('canary', str(error.exception))
        with self.assertRaisesRegex(client.BedrockError, 'invocation failed'):
            self.invoke(code=1)

    def test_configuration_and_deadline_fail_before_request(self):
        for changes in [{'BEDROCK_REVIEW_ENABLED': ''}, {'AWS_REGION': ''}, {'BEDROCK_REVIEW_MODEL': 'other'}]:
            with patch.dict(os.environ, changes), patch('bedrock_client.subprocess.run') as run:
                with self.assertRaises(client.BedrockError):
                    client.complete([], time.monotonic()+10)
                run.assert_not_called()
        with patch('bedrock_client.subprocess.run') as run:
            with self.assertRaises(client.BedrockError):
                client.complete([], time.monotonic()-1)
            run.assert_not_called()

    def test_timeout_does_not_expose_evidence(self):
        with patch('bedrock_client.subprocess.run', side_effect=subprocess.TimeoutExpired('private canary', 1)):
            with self.assertRaisesRegex(client.BedrockError, 'deadline'):
                client.complete([], time.monotonic()+10)

    def test_oversized_request_never_launches(self):
        with patch('bedrock_client.subprocess.run') as run:
            with self.assertRaisesRegex(client.BedrockError, 'request exceeds'):
                client.complete([{'role': 'user', 'content': 'x'*client.MAX_REQUEST_BYTES}], time.monotonic()+10)
            run.assert_not_called()

    def test_packing_estimate_is_local_and_counts_utf8_bytes(self):
        with patch('bedrock_client.subprocess.run') as run:
            self.assertEqual(client.count_tokens([{'role':'user','content':'é'}]), 1154)
            run.assert_not_called()

    def test_timeout_retains_full_context_reservation(self):
        from budget import Ledger, cost, RESERVED_INPUT_TOKENS
        with patch('bedrock_client.subprocess.run', side_effect=subprocess.TimeoutExpired('aws', 1)):
            with self.assertRaises(client.BedrockError):
                client.complete([{'role':'user','content':'diff'}], time.monotonic()+10)
        summary = Ledger(os.environ['REVIEW_BUDGET_LEDGER']).summary()
        self.assertEqual(summary['committed_usd'], cost(RESERVED_INPUT_TOKENS,1024)/1e9)

    def test_only_explicit_truncation_gets_one_larger_retry(self):
        def envelope(stop):
            return {'usage': {'inputTokens': 20, 'outputTokens': 20}, 'stopReason': stop,
                    'output': {'message': {'content': [{'text': '{}'}]}}}
        with patch('bedrock_client.invoke', side_effect=[envelope('max_tokens'), envelope('end_turn')]) as invoke:
            self.assertEqual(client.complete([], time.monotonic()+10), {})
            self.assertEqual([c.args[1]['inferenceConfig']['maxTokens'] for c in invoke.call_args_list], [1024,4096])
        with patch('bedrock_client.invoke', return_value=envelope('max_tokens')) as invoke:
            with self.assertRaises(client.BedrockError):
                client.complete([], time.monotonic()+10)
            self.assertEqual(invoke.call_count, 2)
