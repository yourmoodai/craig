import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from budget import Ledger
from evaluate_model import CASES
from qualify import qualify


class QualificationTests(unittest.TestCase):
    def test_promotion_requires_labels_coverage_and_lower_cost(self):
        for cheaper, accurate, expected in ((True,True,'adaptive'), (False,True,'baseline'), (True,False,'baseline')):
            with self.subTest(cheaper=cheaper,accurate=accurate), tempfile.TemporaryDirectory() as directory:
                ledger = Ledger(directory+'/ledger.db')
                def assess(diff, files, *, strategy):
                    ledger.reserve(50 if strategy == 'adaptive' and cheaper else 100,0,'qualification',strategy)
                    defects = [f for f in files if CASES[int(f.split('/')[1][4:])][3]=='REQUEST_CHANGES']
                    if strategy == 'adaptive' and not accurate:
                        defects = []
                    return {'complete':True,'findings':[{'file':f+':1'} for f in defects]}
                cases = [(case[3], [], 'reviewed fixture behavior') for case in CASES]
                with patch.dict(os.environ, REVIEW_BUDGET_LEDGER=directory+'/ledger.db'), patch('qualify.run_model_reviewer',side_effect=cases), patch('qualify.review_diff',side_effect=assess):
                    result = qualify(directory+'/result.json')
                self.assertTrue(result['qualified'])
                self.assertEqual(result['strategy'], expected)
                self.assertEqual(json.loads(Path(directory+'/result.json').read_text())['strategy'],expected)

    def test_failed_initial_qualification_does_not_compare(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, REVIEW_BUDGET_LEDGER=directory+'/ledger.db'), patch('qualify.run_model_reviewer',return_value=('COMMENT', [], 'review incomplete: access unavailable')), patch('qualify.review_diff') as compare:
            result = qualify(directory+'/result.json')
        self.assertFalse(result['qualified'])
        compare.assert_not_called()

    def test_interrupted_comparison_never_records_qualified(self):
        with tempfile.TemporaryDirectory() as directory:
            cases = [(case[3], [], 'reviewed fixture behavior') for case in CASES]
            with patch.dict(os.environ, REVIEW_BUDGET_LEDGER=directory+'/ledger.db'), \
                    patch('qualify.run_model_reviewer', side_effect=cases), \
                    patch('qualify.review_diff', side_effect=ValueError('interrupted')):
                with self.assertRaises(ValueError):
                    qualify(directory+'/result.json')
            self.assertFalse(json.loads(Path(directory+'/result.json').read_text())['qualified'])
