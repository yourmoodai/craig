"""Assess all primary units; retry saturated batches only by subdivision."""
import time

from bedrock_client import BedrockError, count_tokens
from chunk_plan import anchors, manifest, plan, render, subdivide


def assess_plan(diff, files, context, report, deadline, strategy, dry_run):
    from adversarial_review import review_messages, run_model_reviewer

    def messages(units, support):
        return review_messages(render(units), sorted({u.path for u in units}), context, anchors(units),
                               supporting_context='\nSupporting evidence (context only):\n'+support if support else '')
    try:
        planned = plan(diff, lambda m: count_tokens(m, deadline), messages, strategy=strategy)
    except (ValueError, BedrockError) as error:
        return {**report, 'summary': f'review incomplete: {error}'}
    report['manifest'] = manifest(planned)
    report['subdivisions'] = []
    report['total_units'] = sum(len(c['units']) for c in planned['chunks'])
    report['completed_units'] = 0
    report['total_chunks'] = len(planned['chunks'])
    if dry_run:
        return report
    pending = [(str(i), c['units'], c['support']) for i, c in enumerate(planned['chunks'], 1)]
    attempts = 0
    while pending:
        identity, units, support = pending.pop(0)
        attempts += 1
        if time.monotonic() >= deadline or attempts > 128:
            disposition, findings, summary = 'COMMENT', [], 'review incomplete: execution budget exhausted'
        else:
            disposition, findings, summary = run_model_reviewer(
                render(units), sorted({u.path for u in units}), context, anchors(units),
                deadline=deadline, supporting_context='\nSupporting evidence (context only):\n'+support if support else '')
        if 'subdivision required' in summary and attempts < 128 and time.monotonic() < deadline:
            try:
                children = subdivide(units)
                report['subdivisions'].append({'parent': identity, 'unit_ids': [u.id for u in units], 'reason': summary})
                pending[0:0] = [(identity+'.'+str(i), child, support) for i, child in enumerate(children)]
                report['total_chunks'] += len(children)-1
                report['total_units'] += sum(len(c) for c in children)-len(units)
                continue
            except ValueError:
                summary = 'review incomplete: indivisible change could not be assessed'
        completed = not summary.startswith('review incomplete:')
        report['chunks'].append({'index': identity, 'file': ', '.join(sorted({u.path for u in units})),
                                 'unit_ids': [u.id for u in units], 'disposition': disposition,
                                 'complete': completed, 'summary': summary, 'findings': findings})
        report['findings'].extend(findings)
        if completed:
            report['completed_chunks'] += 1
            report['completed_units'] += len(units)
            report['reviewed_lines'] += sum(len(u.rows) for u in units)
    report['complete'] = not report['omitted_binary_changes'] and report['completed_units'] == report['total_units']
    report['disposition'] = 'REQUEST_CHANGES' if report['findings'] else 'APPROVE' if report['complete'] else 'COMMENT'
    report['summary'] = f"reviewed {report['completed_units']}/{report['total_units']} primary units across {report['completed_chunks']}/{report['total_chunks']} chunks; supporting context is not additional coverage."
    return report
