"""Preserve spending and coverage evidence even when inference is unavailable."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import zipfile

from budget import Ledger, BudgetError
from chunk_plan import ASSESSMENT_LIMIT, bounded_strategy
from reporting import finalize, render, fallback_overview
from qualify import qualify

root = Path(__file__).resolve().parent
path = Path('review-result/report.json')
path.parent.mkdir(exist_ok=True)
head = os.environ['HEAD_SHA']
report = {'commit_id': head, 'complete': False, 'qualified': False,
          'disposition': 'REQUEST_CHANGES', 'findings': [],
          'summary': 'Review unavailable; qualification or assessment did not complete.'}
finalize(report, qualified=False, synthesize=False)
ledger = None
try:
    if os.environ.get('BEDROCK_REVIEW_ENABLED') != 'true':
        raise ValueError('bedrock review is disabled pending model access and qualification')
    os.environ['REVIEW_BUDGET_LEDGER'] = str(Path('review-result/ledger.db').resolve())
    ledger = Ledger(os.environ['REVIEW_BUDGET_LEDGER'])
    archives = list(Path('review-result/previous-ledgers').glob('*.zip'))
    if Path('review-result/previous-ledger.zip').exists():
        archives.append(Path('review-result/previous-ledger.zip'))
    for archive in archives:
        with zipfile.ZipFile(archive) as z:
            if z.getinfo('ledger.db').file_size > 10_000_000:  # 10 MB ledger limit
                raise ValueError('pilot ledger exceeds size limit')
            with tempfile.NamedTemporaryFile(suffix='.db') as source:
                source.write(z.read('ledger.db'))
                source.flush()
                ledger.merge(source.name)
    for recovery in json.loads(Path('review-result/recoveries.json').read_text()):
        ledger.record_recovery(f"{recovery['run']}-{recovery['attempt']}", recovery['nanodollars'])
    ledger.record_preflight()
    ledger.record_external_reservation(int(os.environ.get('BEDROCK_EXTERNAL_PROBE_NANODOLLARS') or '0'))
    # Dry runs estimate packing locally for the exact head.
    for strategy in ('baseline', 'adaptive'):
        subprocess.run([sys.executable, str(root/'adversarial_review.py'), '--base', os.environ['BASE_SHA'],
                        '--head', head, '--strategy', strategy, '--dry-run', '--json-output',
                        f'review-result/{strategy}-manifest.json'], check=True, timeout=2400)
    baseline = json.loads(Path('review-result/baseline-manifest.json').read_text())
    for field in ('files', 'total_lines', 'total_chunks', 'total_units', 'excluded_assets', 'test_evidence'):
        report[field] = baseline[field]
    report['overview'] = fallback_overview(report)
    qualification = qualify('review-result/qualification.json')
    if not qualification['qualified']:
        report['summary'] = 'Live model qualification failed; PR assessment did not start.'
        for case in qualification['cases']:
            if not case['passed']:
                report['review_failures'].append('qualification case failed: '+case['case']+'; '+case['summary'])
        for strategy, comparison in qualification['comparison'].items():
            for batch in comparison['batches']:
                if not batch['passed']:
                    report['review_failures'].append(f"{strategy} qualification failed: missing defects {batch['missing_defects']}; unexpected findings {batch['unexpected_findings']}")
                    for chunk in batch['report'].get('chunks', []):
                        if not chunk.get('complete'):
                            report['review_failures'].append(chunk['summary'])
        raise ValueError('model qualification or baseline comparison did not pass')
    manifests = {s: json.loads(Path(f'review-result/{s}-manifest.json').read_text())['manifest']
                 for s in ('baseline', 'adaptive')}
    strategy = bounded_strategy(qualification['strategy'], manifests)
    Path('review-result/strategy.json').write_text(json.dumps({
        'qualified_preference': qualification['strategy'], 'selected': strategy,
        'assessment_limit': ASSESSMENT_LIMIT}))
    subprocess.run([sys.executable, str(root/'adversarial_review.py'), '--base', os.environ['BASE_SHA'],
                    '--head', head, '--strategy', strategy, '--fail-on', 'none',
                    '--json-output', str(path)], timeout=2400, check=False)
    if not path.exists():
        raise ValueError('review did not produce a report')
    report = json.loads(path.read_text())
except (OSError, ValueError, BudgetError, subprocess.SubprocessError) as error:
    report['review_failures'].append('review execution unavailable: '+type(error).__name__)
finally:
    if ledger is not None:
        report['budget'] = ledger.summary()
        Path('review-result/settled.json').write_text(json.dumps(report['budget'], indent=2)+'\n')
    report['body'] = render(report)
    path.write_text(json.dumps(report, indent=2)+'\n')
    if os.environ.get('GITHUB_STEP_SUMMARY'):
        Path(os.environ['GITHUB_STEP_SUMMARY']).write_text(report['body'])
