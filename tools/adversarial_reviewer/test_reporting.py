import unittest
from unittest.mock import patch
from reporting import finalize, fallback_overview
from kimi_client import KimiError

OVERVIEW = ('This change updates the review pipeline to assess independent code chunks and combine their results into a concise overview. '
            'The publisher derives approval from complete coverage and verified findings, while retaining individual results for inspection. '
            'Documentation images are identified separately from executable changes. Review failures remain visible and require a successful rerun before approval can be granted.')


class ReportingTests(unittest.TestCase):
    def report(self, **changes):
        result=dict(files=2, complete=True, disposition='APPROVE', findings=[], total_chunks=2, completed_chunks=2,
                    chunks=[dict(index=n, file=f'file{n}', complete=True, disposition='APPROVE', summary=f'behavior{n}') for n in [1,2]],
                    excluded_assets=['docs/assets/craig.png'], summary='both chunks reviewed')
        result.update(changes)
        return result

    def test_synthesis_covers_every_chunk_and_precedes_verdict(self):
        with patch('reporting.complete', return_value={'overview':OVERVIEW}) as model:
            report=finalize(self.report(), qualified=True)
        prompt=model.call_args.args[0][1]['content']
        for text in ['behavior1', 'behavior2', 'docs/assets/craig.png']:
            self.assertIn(text,prompt)
        self.assertEqual(report['disposition'],'APPROVE')
        self.assertTrue(50 <= len(report['overview'].split()) <= 100)
        self.assertLess(report['body'].index(OVERVIEW),report['body'].index('**Verdict: Approved**'))
        self.assertIn('<details>',report['body'])
        self.assertIn('not reviewed',report['body'])

    def test_invalid_overview_retries_once(self):
        with patch('reporting.complete', side_effect=[{'overview':'too short'},{'overview':OVERVIEW}]) as model:
            result=finalize(self.report(),qualified=True)
        self.assertEqual(model.call_count,2)
        self.assertEqual(result['disposition'],'APPROVE')

    def test_unavailable_synthesis_fails_closed_with_bounded_overview(self):
        for response in [{'overview':'short'}, {'overview':'word '*101}, None]:
            with patch('reporting.complete', side_effect=KimiError('timeout') if response is None else None, return_value=response) as model:
                result=finalize(self.report(),qualified=True)
            self.assertEqual(model.call_count,2)
            self.assertEqual(result['disposition'],'REQUEST_CHANGES')
            self.assertTrue(50 <= len(result['overview'].split()) <= 100)
            self.assertEqual(result['findings'],[])

    def test_model_prose_cannot_override_findings_or_coverage(self):
        for changes in [dict(complete=False),dict(findings=[dict(file='x:1',title='defect',details='fix boundary')]),dict(disposition='COMMENT')]:
            with patch('reporting.complete',return_value={'overview':OVERVIEW}):
                result=finalize(self.report(**changes),qualified=True)
            self.assertEqual(result['disposition'],'REQUEST_CHANGES')

    def test_failed_qualification_never_calls_model(self):
        with patch('reporting.complete') as model:
            result=finalize(self.report(),qualified=False)
        model.assert_not_called()
        self.assertEqual(result['disposition'],'REQUEST_CHANGES')

    def test_oversized_synthesis_never_silently_truncates(self):
        report=self.report(summary='x'*120001)
        with patch('reporting.complete') as model:
            finalize(report,qualified=True)
        model.assert_not_called()
        self.assertEqual(report['disposition'],'REQUEST_CHANGES')
