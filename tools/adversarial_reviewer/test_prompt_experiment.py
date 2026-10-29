import json
import os
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import patch

from adversarial_review import SYSTEM_PROMPT, VERIFICATION_PROMPT
from bedrock_client import complete, BedrockError, MODEL
from budget import Ledger
from diagnostics import capture
from prompt_experiment import Limits, ExperimentStopped, run
from prompt_profiles import select, prompts, settings


class PromptExperimentTests(unittest.TestCase):
    def test_prompt_selection_restores_production_and_preserves_contracts(self):
        with select('current'):
            self.assertEqual(prompts(SYSTEM_PROMPT,VERIFICATION_PROMPT),(SYSTEM_PROMPT,VERIFICATION_PROMPT))
        with select('candidate',0.7):
            system, verification=prompts(SYSTEM_PROMPT,VERIFICATION_PROMPT)
            self.assertIn('For Savior,'+SYSTEM_PROMPT.split('For Savior,')[1].split('\n\nThe diff')[0],system)
            self.assertTrue(system.startswith(SYSTEM_PROMPT))
            self.assertTrue(verification.startswith(VERIFICATION_PROMPT))
            self.assertIn('Remove unsupported claims',verification)
        self.assertEqual(settings.get(),('candidate',0,None))

    def test_truncated_envelope_is_captured_before_rejection(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {
                'REVIEW_BUDGET_LEDGER':directory+'/ledger.db', 'AWS_REGION':'us-east-1',
                'BEDROCK_REVIEW_ENABLED':'true','BEDROCK_REVIEW_MODEL':MODEL}):
            envelope={'usage':{'inputTokens':20,'outputTokens':4096},'stopReason':'max_tokens',
                      'output':{'message':{'content':[{'text':'partial synthetic JSON'}]}}}
            with capture() as events, patch('bedrock_client.invoke',return_value=envelope):
                with self.assertRaises(BedrockError):
                    complete([{'role':'user','content':'synthetic'}],time.monotonic()+10)
            self.assertEqual(events[0]['response'],envelope)
            self.assertEqual(events[0]['inference_config']['maxTokens'],1024)

    def test_limits_stop_before_another_reservation(self):
        with tempfile.TemporaryDirectory() as directory:
            ledger=Ledger(directory+'/ledger.db');limits=Limits(ledger)
            with self.assertRaises(ExperimentStopped): limits.reserve(150_000_001)
            self.assertEqual(ledger.summary()['calls'],[])
            limits.calls=96
            with self.assertRaises(ExperimentStopped): limits.reserve(1)
            limits.calls=0;limits.deadline=0
            with self.assertRaises(ExperimentStopped): limits.reserve(1)

    def test_failed_arms_finish_all_cases_then_stop_without_confirmation(self):
        with tempfile.TemporaryDirectory() as directory:
            output=Path(directory)/'out'
            with patch('prompt_experiment.run_model_reviewer',return_value=('COMMENT',[],'review incomplete: invalid JSON')), patch('prompt_experiment.qualify') as confirm:
                report=run(output,Ledger(Path(directory)/'ledger.db'))
            self.assertEqual(len(report['cases']),30)
            self.assertFalse(report['eligible'])
            confirm.assert_not_called()
            self.assertEqual(json.loads((output/'results.json').read_text())['metrics']['candidate-0.7']['incomplete'],10)

    def test_correct_candidate_requires_second_live_qualification(self):
        from evaluate_model import CASES
        answers=[]
        for c in CASES: answers.extend([(c[3],[],'synthetic result')]*2)
        with tempfile.TemporaryDirectory() as directory, patch('prompt_experiment.run_model_reviewer',side_effect=answers), patch('prompt_experiment.qualify',return_value={'qualified':False}) as confirm:
            result=run(Path(directory)/'out',Ledger(Path(directory)/'ledger.db'))
        self.assertEqual(len(result['cases']),20)
        confirm.assert_called_once()
        self.assertFalse(result['eligible'])

    def test_experiment_limit_prevents_provider_call_and_reservation(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {
                'REVIEW_BUDGET_LEDGER':directory+'/ledger.db', 'AWS_REGION':'us-east-1',
                'BEDROCK_REVIEW_ENABLED':'true','BEDROCK_REVIEW_MODEL':MODEL}):
            def stop(amount):
                raise ExperimentStopped('stop')
            with select('candidate',0.7,stop), patch('bedrock_client.invoke') as invoke:
                with self.assertRaises(ExperimentStopped):
                    complete([{'role':'user','content':'synthetic'}],time.monotonic()+10)
                invoke.assert_not_called()
            self.assertEqual(Ledger(directory+'/ledger.db').summary()['calls'],[])
