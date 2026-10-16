"""Preserve spending and coverage evidence even when inference is unavailable."""
import json
import os
from pathlib import Path
import subprocess
import sys
import zipfile

from budget import Ledger
from chunk_plan import bounded_strategy
from reporting import finalize, render
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
    archive = Path('review-result/previous-ledger.zip')
    if archive.exists():
        with zipfile.ZipFile(archive) as z:
            data = z.read('ledger.db')
            if len(data) > 10_000_000:  # 10 MB ledger limit
                raise ValueError('pilot ledger exceeds size limit')
            Path('review-result/ledger.db').write_bytes(data)
    os.environ['REVIEW_BUDGET_LEDGER'] = str(Path('review-result/ledger.db').resolve())
    ledger = Ledger(os.environ['REVIEW_BUDGET_LEDGER'])
    ledger.record_preflight()
    # Dry runs estimate packing locally for the exact head.
    for strategy in ('baseline', 'adaptive'):
        subprocess.run([sys.executable, str(root/'adversarial_review.py'), '--base', os.environ['BASE_SHA'],
                        '--head', head, '--strategy', strategy, '--dry-run', '--json-output',
                        f'review-result/{strategy}-manifest.json'], check=True, timeout=2400)
    qualification = qualify('review-result/qualification.json')
    if not qualification['qualified']:
        raise ValueError('model qualification or baseline comparison did not pass')
    manifests = {s: json.loads(Path(f'review-result/{s}-manifest.json').read_text())['manifest']
                 for s in ('baseline', 'adaptive')}
    strategy = bounded_strategy(qualification['strategy'], manifests)
    Path('review-result/strategy.json').write_text(json.dumps({
        'qualified_preference': qualification['strategy'], 'selected': strategy,
        'assessment_limit': 128}))
    subprocess.run([sys.executable, str(root/'adversarial_review.py'), '--base', os.environ['BASE_SHA'],
                    '--head', head, '--strategy', strategy, '--fail-on', 'none',
                    '--json-output', str(path)], timeout=2400, check=False)
    if not path.exists():
        raise ValueError('review did not produce a report')
    report = json.loads(path.read_text())
except (OSError, ValueError, subprocess.SubprocessError) as error:
    report['review_failures'].append('review execution unavailable: '+type(error).__name__)
finally:
    if ledger is not None:
        report['budget'] = ledger.summary()
        Path('review-result/settled.json').write_text(json.dumps(report['budget'], indent=2)+'\n')
    report['body'] = render(report)
    path.write_text(json.dumps(report, indent=2)+'\n')
    if os.environ.get('GITHUB_STEP_SUMMARY'):
        Path(os.environ['GITHUB_STEP_SUMMARY']).write_text(report['body'])
