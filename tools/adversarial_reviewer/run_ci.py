"""Publishable advisory results even when qualification or inference is unavailable."""
import json
import os
from pathlib import Path
import subprocess
import sys

root = Path(__file__).resolve().parent
path = Path('review-result/report.json')
path.parent.mkdir(exist_ok=True)
head = os.environ['HEAD_SHA']
report = {'commit_id': head, 'complete': False, 'qualified': False,
          'disposition': 'COMMENT', 'findings': [],
          'body': 'Review incomplete: reviewer qualification or credentials unavailable. Maintainer review is required.'}
try:
    qualified = subprocess.run([sys.executable, str(root/'evaluate_model.py')], timeout=1000).returncode == 0
    if qualified:
        subprocess.run([sys.executable, str(root/'adversarial_review.py'), '--base', os.environ['BASE_SHA'],
                        '--head', head, '--fail-on', 'none', '--json-output', str(path)], timeout=1900)
        if path.exists():
            report = json.loads(path.read_text())
            report['qualified'] = True
except (OSError, ValueError, subprocess.TimeoutExpired):
    pass
path.write_text(json.dumps(report, indent=2)+'\n')
if os.environ.get('GITHUB_STEP_SUMMARY'):
    Path(os.environ['GITHUB_STEP_SUMMARY']).write_text(report['body'])
