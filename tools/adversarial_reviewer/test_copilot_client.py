import json
import os
import subprocess
import time
import unittest
from unittest.mock import patch
import copilot_client as client


class CopilotClientTests(unittest.TestCase):
    def setUp(self):
        client._exhausted.clear()
        self.env = patch.dict(os.environ, {'COPILOT_BILLING_VERIFIED':'true', 'COPILOT_REVIEW_MODEL':'configured-model', 'GITHUB_TOKEN':'synthetic-token'})
        self.env.start()
        self.addCleanup(self.env.stop)
        self.addCleanup(client._exhausted.clear)

    def invoke(self, output='{}', code=0, stderr=''):
        with patch('copilot_client.shutil.which',return_value='/pinned/copilot'), patch('copilot_client.subprocess.run',return_value=subprocess.CompletedProcess([],code,output,stderr)) as run:
            value = client.complete([{'role':'user','content':'synthetic diff'}],time.monotonic()+10)
            return value, run.call_args

    def test_isolated_tool_free_call(self):
        value, call = self.invoke('{"findings":[]}')
        self.assertEqual(value, {'findings':[]})
        argv = call.args[0]
        self.assertEqual(argv[argv.index('--available-tools')+1], '--prompt')
        for flag in ['--no-auto-update','--no-custom-instructions','--disable-builtin-mcps','--no-ask-user']:
            self.assertIn(flag,argv)
        self.assertEqual(call.kwargs['env']['HOME'],call.kwargs['cwd'])
        self.assertNotIn('KIMI_API_KEY',call.kwargs['env'])
        self.assertLessEqual(call.kwargs['timeout'],10)

    def test_billing_gate_never_launches(self):
        with patch.dict(os.environ,{'COPILOT_BILLING_VERIFIED':''}), patch('copilot_client.subprocess.run') as run:
            with self.assertRaisesRegex(client.CopilotError,'unverified'):
                client.complete([],time.monotonic()+10)
            run.assert_not_called()

    def test_malformed_truncated_and_oversized_outputs(self):
        for output in ['{"findings":', '[]', 'private canary', 'x'*(client.MAX_RESPONSE_BYTES+1)]:
            with self.assertRaises(client.CopilotError) as error:
                self.invoke(output)
            self.assertNotIn('canary',str(error.exception))

    def test_quota_stops_subsequent_calls(self):
        with self.assertRaisesRegex(client.CopilotError,'quota exhausted'):
            self.invoke(code=1,stderr='quota exceeded private canary')
        with patch('copilot_client.subprocess.run') as run:
            with self.assertRaisesRegex(client.CopilotError,'quota exhausted'):
                client.complete([],time.monotonic()+10)
            run.assert_not_called()

    def test_timeout_and_execution_errors_are_sanitized(self):
        with self.assertRaisesRegex(client.CopilotError,'execution failed'):
            self.invoke(code=1,stderr='private canary')
        with patch('copilot_client.shutil.which',return_value='/pinned/copilot'), patch('copilot_client.subprocess.run',side_effect=subprocess.TimeoutExpired('private canary',10)):
            with self.assertRaisesRegex(client.CopilotError,'deadline'):
                client.complete([],time.monotonic()+10)
