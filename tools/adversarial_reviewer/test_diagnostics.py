"""Qualification failures remain inspectable without capturing production prompts."""
import json
import os
from pathlib import Path
import runpy
import tempfile
import unittest
from unittest.mock import patch

from adversarial_review import run_model_reviewer
from diagnostics import capture, record
from evaluate_model import CASES
from qualify import fixture


class DiagnosticTests(unittest.TestCase):
    def test_new_verification_finding_requires_fresh_assessment(self):
        finding = {'location': 1, 'invariant': 'sanitization must fail closed',
                   'scenario': 'a sanitizer error returns the raw input',
                   'correction': 'propagate the sanitizer exception'}
        response = {'rationale': 'Two distinct changes violate the invariant.',
                    'abstention': '', 'findings': [finding, {**finding, 'location': 2}]}
        anchors = [{'file': 'submit.py', 'line': n, 'evidence': 'return raw'} for n in (1, 2)]
        with patch('adversarial_review.complete', return_value=response):
            disposition, findings, summary = run_model_reviewer(
                'synthetic diff', ['submit.py'], '', anchors, candidate=[finding])
        self.assertEqual(disposition, 'COMMENT')
        self.assertEqual(findings, [])
        self.assertIn('subdivision required', summary)

    def test_overfull_findings_request_subdivision_and_preserve_shape(self):
        diff, filename = fixture(5, CASES[5])
        response = {'rationale': 'The changed divisor raises an exception.', 'abstention': '',
                    'findings': [{'location': 2}]*4}
        with capture() as events, patch('adversarial_review.complete', return_value=response):
            disposition, findings, summary = run_model_reviewer(diff, [filename], '')
        self.assertEqual(disposition, 'COMMENT')
        self.assertEqual(findings, [])
        self.assertIn('subdivision required', summary)
        self.assertEqual(events[0]['response'], response)
        self.assertIn('saturated (4)', events[1]['validation_error'])
        record(response='must not enter the completed qualification capture')
        self.assertEqual(len(events), 2)

    def test_non_array_findings_do_not_masquerade_as_saturation(self):
        diff, filename = fixture(5, CASES[5])
        with patch('adversarial_review.complete', return_value={
                'rationale': 'The changed divisor raises an exception.', 'abstention': '', 'findings': {}}):
            _, _, summary = run_model_reviewer(diff, [filename], '')
        self.assertIn('expected array', summary)
        self.assertNotIn('subdivision required', summary)

    def test_ci_failure_preserves_coverage_and_qualification_reason(self):
        script = Path(__file__).with_name('run_ci.py').resolve()
        with tempfile.TemporaryDirectory() as directory:
            previous = Path.cwd()
            self.addCleanup(os.chdir, previous)
            os.chdir(directory)
            def dry_run(argv, **kwargs):
                Path(argv[-1]).write_text(json.dumps({
                    'files': 162, 'total_lines': 26939, 'total_chunks': 100,
                    'total_units': 258, 'excluded_assets': [], 'test_evidence': {'head': 'test'}}))
            result = {'qualified': False, 'cases': [], 'comparison': {'baseline': {'batches': [{
                'passed': False, 'missing_defects': ['qualification/case2/bridge/submit.py'],
                'unexpected_findings': [], 'report': {'chunks': [{
                    'complete': False, 'summary': 'review incomplete: invalid findings: expected array.'}]}}]}}}
            with patch.dict(os.environ, {'HEAD_SHA': 'test', 'BASE_SHA': 'base',
                                          'BEDROCK_REVIEW_ENABLED': 'true', 'GITHUB_STEP_SUMMARY': '',
                                          'REVIEW_BUDGET_LEDGER': ''}), \
                    patch('subprocess.run', side_effect=dry_run) as run, \
                    patch('qualify.qualify', return_value=result):
                runpy.run_path(str(script), run_name='__main__')
            report = json.loads(Path('review-result/report.json').read_text())
            self.assertEqual(run.call_count, 2)
            self.assertFalse(report['qualified'])
            self.assertIn('162 changed files', report['overview'])
            self.assertIn('0 of 100', report['overview'])
            self.assertIn('case2/bridge/submit.py', '\n'.join(report['review_failures']))
            self.assertIn('expected array', '\n'.join(report['review_failures']))
