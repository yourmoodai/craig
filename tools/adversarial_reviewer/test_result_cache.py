import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from result_cache import get, key, put
from adversarial_review import run_model_reviewer

DIFF = 'diff --git a/a.py b/a.py\n--- a/a.py\n+++ b/a.py\n@@ -1 +1 @@\n-old\n+new\n'


class CacheTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.directory = directory.name
        env = patch.dict(os.environ, REVIEW_RESULT_CACHE=directory.name)
        env.start()
        self.addCleanup(env.stop)

    def test_validated_result_reused_but_changed_context_misses(self):
        with patch('adversarial_review._run_model_reviewer', return_value=('APPROVE', [], 'No concrete defect found.')) as model:
            run_model_reviewer(DIFF, ['a.py'], 'pending checks')
            run_model_reviewer(DIFF, ['a.py'], 'pending checks')
            self.assertEqual(model.call_count, 1)
            run_model_reviewer(DIFF, ['a.py'], 'passed checks')
            run_model_reviewer(DIFF, ['a.py'], 'passed checks', supporting_context='new dependency')
            self.assertEqual(model.call_count, 3)

    def test_incomplete_results_are_not_cached(self):
        with patch('adversarial_review._run_model_reviewer', return_value=('COMMENT', [], 'review incomplete: missing context')) as model:
            for _ in range(2):
                run_model_reviewer(DIFF, ['a.py'], '')
            self.assertEqual(model.call_count, 2)

    def test_expiry_corruption_and_version_change(self):
        identity = key('test', ['settings'])
        with patch('result_cache.time.time', return_value=100):
            put(identity, {'value': 1})
        with patch('result_cache.time.time', return_value=200):
            self.assertIsNone(get(identity, 99))
            self.assertEqual(get(identity, 101), {'value': 1})
        with patch('result_cache.fingerprint', return_value='changed'):
            self.assertNotEqual(key('test', ['settings']), identity)
        (Path(self.directory)/(identity+'.json')).write_text('broken')
        self.assertIsNone(get(identity, 86400))

    def test_qualified_cache_is_reused_without_calling_model(self):
        from qualify import qualify
        from budget import Ledger
        ledger = str(Path(self.directory)/'ledger.db')
        Ledger(ledger)
        value = {'qualified': True, 'cases': [{'passed': True}],
                 'comparison': {'baseline': {'passed': True}}, 'strategy': 'baseline'}
        with patch.dict(os.environ, REVIEW_BUDGET_LEDGER=ledger), patch('qualify._qualify', return_value=value) as model:
            qualify(str(Path(self.directory)/'qualification.json'))
            result = qualify(str(Path(self.directory)/'qualification.json'))
            self.assertEqual(model.call_count, 1)
            self.assertTrue(result['cache_hit'])

    def test_zip_restore_ignores_path_traversal_and_expired_entries(self):
        import zipfile
        from result_cache import restore
        archive = Path(self.directory)/'cache.zip'
        identity = key('test', [])
        with zipfile.ZipFile(archive, 'w') as zipped:
            zipped.writestr('../escape.json', '{}')
            zipped.writestr(identity+'.json', json.dumps({'key': identity, 'created': 0, 'value': 'expired'}))
        restore([archive])
        self.assertIsNone(get(identity, 7*86400))
        self.assertFalse((Path(self.directory).parent/'escape.json').exists())
