"""Coverage, coordinates and bounded subdivision are deterministic contracts."""
import unittest
from unittest.mock import patch

from adversarial_review import review_diff, review_messages
from chunk_plan import anchors, ceiling, manifest, parse, plan, render


def change(path='a.py', body='-old\n+new\n', old=1, new=1):
    return f'diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -{old},1 +{new},1 @@\n'+body


def count(messages):
    return sum(len(m['content'].encode()) for m in messages)//3


def messages(units, support):
    return review_messages(render(units), [u.path for u in units], 'passing tests', anchors(units), supporting_context=support)


class ChunkPlanTests(unittest.TestCase):
    def test_thresholds(self):
        self.assertEqual([ceiling(n) for n in (8,9,32,33)], [4000,8000,8000,16000])

    def test_regrouping_and_splitting_preserve_unique_evidence_and_coordinates(self):
        original = parse(change('z.py', '-é\n+你好\n context\n-remove\n+replace\n', 10, 20)+change('a.py'))
        before = anchors(original)
        children = original[0].split()+original[1:]
        self.assertEqual(anchors(children), before)
        after = anchors(list(reversed(original)))
        self.assertEqual({a['id'] for a in before}, {a['id'] for a in after})
        self.assertEqual(len(before), len({a['id'] for a in before}))
        self.assertEqual([(a['side'],a['line']) for a in before[:3]], [('old',10),('new',20),('new',21)])
        self.assertEqual(''.join(u.rows[0] for u in original[:1]), '-é\n')

    def test_deletion_and_metadata_have_review_locations(self):
        deletion = change(body='-removed\n').replace('+++ b/a.py','+++ /dev/null')
        units = parse(deletion+'diff --git a/x b/y\nsimilarity index 100%\nrename from x\nrename to y\n')
        self.assertEqual(anchors(units)[0]['side'], 'old')
        self.assertEqual(anchors(units)[1]['side'], 'file')
        self.assertEqual(units[1].path, 'y')

    def test_primary_ownership_limits_and_manifest(self):
        diff = ''.join(change(f'dir/{n}.py') for n in range(33))
        planned = plan(diff, count, messages)
        all_units = [u for c in planned['chunks'] for u in c['units']]
        self.assertEqual({u.id for u in all_units}, {u.id for u in parse(diff)})
        self.assertEqual(len(all_units),33)
        for chunk in planned['chunks']:
            self.assertLessEqual(len(chunk['units']),16)
            self.assertLessEqual(len({u.path for u in chunk['units']}),8)
            self.assertLessEqual(chunk['input_tokens'],48000)
        self.assertEqual(manifest(planned)['original_units'],33)

    def test_support_never_repeats_primary_units(self):
        planned = plan(change('a.py', '+import b\n')+change('b.py'), count, messages)
        self.assertEqual(len(planned['chunks']),1)
        self.assertEqual(planned['chunks'][0]['support'],'')

    def test_indivisible_line_fails_without_truncation(self):
        with self.assertRaisesRegex(ValueError,'indivisible'):
            plan(change(body='+'+'x'*200000+'\n'),count,messages)

    def test_four_defects_are_reassessed_after_saturation(self):
        diff = ''.join(change(f'{n}.py') for n in range(4))
        calls = []
        def assess(text, files, context, evidence, **kwargs):
            calls.append(files)
            if len(files)>2:
                return 'COMMENT',[], 'review incomplete: findings saturated; subdivision required.'
            return 'REQUEST_CHANGES',[{'file':f+':1'} for f in files], 'verified concrete failures'
        with patch('adaptive_review.count_tokens', side_effect=lambda m,d: count(m)), patch('adversarial_review.run_model_reviewer',side_effect=assess):
            report = review_diff(diff,[f'{n}.py' for n in range(4)],strategy='adaptive')
        self.assertTrue(report['complete'])
        self.assertEqual(len(report['findings']),4)
        self.assertEqual(len(calls),3)
        self.assertEqual(report['completed_units'],4)

    def test_count_failure_prevents_model_calls(self):
        from bedrock_client import BedrockError
        with patch('adaptive_review.count_tokens',side_effect=BedrockError('unavailable')), patch('adversarial_review.run_model_reviewer') as model:
            report = review_diff(change(),['a.py'],strategy='adaptive')
        self.assertFalse(report['complete'])
        model.assert_not_called()

    def test_new_verification_defects_are_reassessed_in_children(self):
        diff = change('a.py') + change('b.py')
        def assess(text, files, context, evidence, **kwargs):
            if len(files) > 1:
                return 'COMMENT', [], 'review incomplete: verification introduced a new finding; subdivision required.'
            return 'REQUEST_CHANGES', [{'file': files[0]+':1'}], 'verified concrete failure'
        with patch('adversarial_review.run_model_reviewer', side_effect=assess) as model:
            report = review_diff(diff, ['a.py', 'b.py'], strategy='baseline')
        self.assertTrue(report['complete'])
        self.assertEqual(model.call_count, 3)
        self.assertEqual(len(report['findings']), 2)
        self.assertEqual(len(report['subdivisions']), 1)

    def test_full_plan_must_fit_even_after_fixture_promotion(self):
        from chunk_plan import bounded_strategy
        manifests = {'baseline': {'chunks': [None]*67}, 'adaptive': {'chunks': [None]*167}}
        self.assertEqual(bounded_strategy('adaptive', manifests), 'baseline')
        manifests['adaptive']['chunks'] = [None]*50
        self.assertEqual(bounded_strategy('adaptive', manifests), 'adaptive')
        manifests['baseline']['chunks'] = [None]*129
        with self.assertRaises(ValueError):
            bounded_strategy('baseline', manifests)
