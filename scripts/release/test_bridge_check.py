import subprocess
import unittest
from unittest.mock import patch
from bridge_check import request

class BridgeCheckTests(unittest.TestCase):
    def test_diagnostics_do_not_include_response_or_input(self):
        cases=[('',0,'empty_output'),('synthetic private canary',0,'invalid_json'),('{}',0,'invalid_response'),('synthetic private canary',3,'exit_3')]
        for output,code,reason in cases:
            with patch('bridge_check.subprocess.run',return_value=subprocess.CompletedProcess([],code,output,'synthetic private canary')):
                with self.assertRaisesRegex(RuntimeError,'status: bridge_'+reason) as raised:
                    request(['node'],{'private':'synthetic private canary'},{},'status')
                self.assertNotIn('canary',str(raised.exception))
    def test_timeout_is_classified(self):
        with patch('bridge_check.subprocess.run',side_effect=subprocess.TimeoutExpired('node',15)):
            with self.assertRaisesRegex(RuntimeError,'status: bridge_timeout'):
                request(['node'],{}, {},'status')
    def test_valid_envelope_returns_value(self):
        with patch('bridge_check.subprocess.run',return_value=subprocess.CompletedProcess([],0,'{"version":1,"status":"ok","value":{"ready":true}}','')):
            self.assertEqual(request(['node'],{}, {},'status'),{'ready':True})
