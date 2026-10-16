"""Promote adaptive packing only after matched fixtures preserve labels at lower cost."""
import json
import os
from pathlib import Path
import time

from adversarial_review import review_diff, run_model_reviewer
from budget import Ledger
from diagnostics import capture
from evaluate_model import CASES


def fixture(index, case):
    name, filename, change, expected = case
    filename = f'qualification/case{index}/'+filename
    old = sum(not line.startswith('+') for line in change.splitlines())
    new = sum(not line.startswith('-') for line in change.splitlines())
    return (f'diff --git a/{filename} b/{filename}\n--- a/{filename}\n+++ b/{filename}\n'
            f'@@ -1,{old} +1,{new} @@\n'+change), filename


def qualify(output):
    from result_cache import key, get, put, measure
    from bedrock_client import MODEL
    from prompt_profiles import settings
    identity = key('qualification', [MODEL, settings.get()[:2], CASES])
    cached = get(identity, 86400)
    if isinstance(cached, dict) and cached.get('qualified') is True and cached.get('cases') and all(c.get('passed') is True for c in cached['cases']) and cached.get('comparison', {}).get('baseline', {}).get('passed') is True:
        measure('hit')
        cached = {**cached, 'cache_hit': True, 'budget': Ledger(os.environ['REVIEW_BUDGET_LEDGER']).summary()}
        Path(output).write_text(json.dumps(cached, indent=2)+'\n')
        return cached
    with capture() as events:
        # Qualification must exercise the model, not cached fixture assessments.
        cache = os.environ.get('REVIEW_CACHE_DISABLED')
        os.environ['REVIEW_CACHE_DISABLED'] = 'true'
        try:
            result = _qualify(output, events)
        finally:
            if cache is not None:
                os.environ['REVIEW_CACHE_DISABLED'] = cache
            else:
                os.environ.pop('REVIEW_CACHE_DISABLED', None)
        if result['qualified']:
            put(identity, result)
        return result


def _qualify(output, events):
    ledger = Ledger(os.environ['REVIEW_BUDGET_LEDGER'])
    os.environ['REVIEW_BUDGET_PHASE'] = 'qualification'
    result = {'qualified': False, 'strategy': 'baseline', 'cases': [], 'comparison': {}, 'diagnostics': events}
    try:
        for index, case in enumerate(CASES):
            diff, filename = fixture(index, case)
            actual, findings, summary = run_model_reviewer(diff, [filename], '')
            passed = actual == case[3] and not summary.startswith('review incomplete:')
            result['cases'].append({'case': case[0], 'passed': passed, 'summary': summary, 'findings': findings, 'expected': case[3], 'actual': actual})
            if not passed:
                return result
        for strategy in ('baseline', 'adaptive'):
            before = ledger.summary()['committed_usd']
            prior_calls = len(ledger.summary()['calls'])
            began = time.monotonic()
            batches = []
            for disposition in ('APPROVE', 'REQUEST_CHANGES'):
                fixtures = [fixture(i, c) for i, c in enumerate(CASES) if c[3] == disposition]
                report = review_diff(''.join(d for d, _ in fixtures), [f for _, f in fixtures], strategy=strategy)
                found = {finding['file'].rsplit(':', 1)[0] for finding in report['findings']}
                expected = set() if disposition == 'APPROVE' else {f for _, f in fixtures}
                batches.append({'passed': report['complete'] and found == expected, 'report': report,
                                'missing_defects': sorted(expected-found), 'unexpected_findings': sorted(found-expected)})
            truncated = any((c.get('metrics') or {}).get('stop_reason') == 'max_tokens' for c in ledger.summary()['calls'][prior_calls:])
            result['comparison'][strategy] = {'batches': batches, 'cost_usd': ledger.summary()['committed_usd']-before,
                                               'seconds': time.monotonic()-began,
                                               'truncated': truncated,
                                               'passed': not truncated and all(b['passed'] for b in batches)}
        baseline, adaptive = (result['comparison'][key] for key in ('baseline', 'adaptive'))
        result['qualified'] = baseline['passed']
        if baseline['passed'] and adaptive['passed'] and adaptive['cost_usd'] < baseline['cost_usd']:
            result['strategy'] = 'adaptive'
        return result
    finally:
        result['budget'] = ledger.summary()
        Path(output).write_text(json.dumps(result, indent=2)+'\n')
        os.environ['REVIEW_BUDGET_PHASE'] = 'review'
