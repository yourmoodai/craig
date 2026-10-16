"""Compare fixed synthetic cases locally under one restored pilot ledger."""
import argparse
import json
import os
from pathlib import Path
import time

from adversarial_review import run_model_reviewer
from budget import Ledger, cost
from diagnostics import capture
from evaluate_model import CASES
from evidence import repetitive
from prompt_profiles import select
from qualify import fixture, qualify


class ExperimentStopped(Exception):
    pass


class Limits:
    def __init__(self, ledger):
        self.ledger = ledger
        self.before = ledger.summary()['committed_usd']
        self.deadline = time.monotonic()+900
        self.calls = 0

    def reserve(self, amount):
        if time.monotonic() >= self.deadline or self.calls >= 96:
            raise ExperimentStopped('execution limit reached')
        if self.ledger.summary()['committed_usd']-self.before+amount/1e9 > 0.15:
            raise ExperimentStopped('experiment spending limit reached')
        self.calls += 1


def metrics(rows):
    events = [e for r in rows for e in r['diagnostics'] if e['stage']=='transport']
    text = lambda e: ''.join(b.get('text','') for b in e['response'].get('output',{}).get('message',{}).get('content',[]))
    return {'cases':len(rows), 'correct':sum(r['passed'] for r in rows),
            'false_positives':sum(r['expected']=='APPROVE' and r['actual']=='REQUEST_CHANGES' for r in rows),
            'missed_defects':sum(r['expected']=='REQUEST_CHANGES' and r['actual']=='APPROVE' for r in rows),
            'incomplete':sum(r['actual']=='COMMENT' for r in rows),
            'verification_dismissals':sum(r['expected']=='REQUEST_CHANGES' and r['actual']=='APPROVE' and any(e['stage']=='verification' for e in r['diagnostics']) for r in rows),
            'truncated':sum(e['response'].get('stopReason')=='max_tokens' for e in events),
            'repetitive':sum(repetitive(text(e)) for e in events),
            'invalid_responses':sum(r['summary'].startswith('review incomplete:') and any(word in r['summary'] for word in ('invalid', 'repetitive', 'incomplete finding', 'missing or')) for r in rows),
            'actual_usd':sum(cost(e['response'].get('usage',{}).get('inputTokens',0),e['response'].get('usage',{}).get('outputTokens',0)) for e in events)/1e9,
            'input_tokens':sum(e['response'].get('usage',{}).get('inputTokens',0) for e in events),
            'output_tokens':sum(e['response'].get('usage',{}).get('outputTokens',0) for e in events),
            'seconds':round(sum(e['seconds'] for e in events),3)}


def run(output, ledger):
    output.mkdir(exist_ok=False)
    limits = Limits(ledger)
    result = {'cases':[], 'eligible':False, 'confirmation':None}
    def save():
        result['metrics'] = {arm:metrics([r for r in result['cases'] if r['arm']==arm]) for arm in sorted({r['arm'] for r in result['cases']})}
        result['budget'] = ledger.summary()
        result['additional_committed_usd'] = result['budget']['committed_usd']-limits.before
        (output/'results.json').write_text(json.dumps(result,indent=2)+'\n')
    def assess(index, name, temperature):
        case=CASES[index]; diff, filename=fixture(index,case)
        with select(name,temperature,limits.reserve), capture() as events:
            actual, findings, summary=run_model_reviewer(diff,[filename],'',deadline=limits.deadline)
        row={'case':case[0], 'index':index, 'arm':f'{name}-{temperature}', 'expected':case[3],
             'actual':actual, 'passed':actual==case[3] and not summary.startswith('review incomplete:'),
             'findings':findings, 'summary':summary, 'diagnostics':events}
        result['cases'].append(row);save()
        print(json.dumps({k:row[k] for k in ('arm','case','passed','actual')}),flush=True)
    try:
        for index in range(len(CASES)):
            for name in (('current','candidate') if index%2==0 else ('candidate','current')):
                assess(index,name,0)
        temperature=0
        if not all(r['passed'] for r in result['cases'] if r['arm']=='candidate-0'):
            temperature=0.7
            for index in range(len(CASES)):
                assess(index,'candidate',temperature)
        chosen=[r for r in result['cases'] if r['arm']==f'candidate-{temperature}']
        if len(chosen)==len(CASES) and all(r['passed'] for r in chosen):
            with select('candidate',temperature,limits.reserve):
                result['confirmation']=qualify(output/'confirmation.json')
            result['eligible']=result['confirmation']['qualified']
            result['selected_temperature']=temperature
    except ExperimentStopped as error:
        result['stopped']=str(error)
    finally:
        save()
    return result


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ledger',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    if not args.ledger.is_file():
        parser.error('restore the shared pilot ledger before running')
    if os.environ.get('AWS_PROFILE')!='awill' or os.environ.get('AWS_REGION')!='us-east-1':
        parser.error('explicit awill profile and us-east-1 region required')
    os.environ.update(REVIEW_BUDGET_LEDGER=str(args.ledger.resolve()),REVIEW_BUDGET_PHASE='qualification')
    run(args.output,Ledger(args.ledger))
