import {json, save, digest} from './corpus.mjs';
import {readFile, writeFile} from 'node:fs/promises';
import {resolve} from 'node:path';
import {replacementSummary} from './quality.mjs';
import {sameIdentity} from './common.mjs';

const percent = value => value===null ? 'unavailable' : `${(value*100).toFixed(1)}%`;
const ms = value => Number.isFinite(value) ? value>=1000?`${(value/1000).toFixed(2)} s`:`${value.toFixed(2)} ms` : 'unavailable';
const escape = value => String(value).replaceAll('|','\\|').replaceAll('\n',' ').replaceAll('`','');
export function validateReports(reports) {
  if (reports.length!==3 || new Set(reports.map(r=>r.kind)).size!==3 || reports.some(r=>!['quality','e2e','performance'].includes(r.kind))) throw Error('missing_reports');
  const first = reports[0].identity;
  for (const report of reports) {
    if (!['completed','failed'].includes(report.status) || !report.identity || report.version!==1) throw Error('incomplete_report');
    if(report.identity_stable===true && (!report.identity_end || !sameIdentity(report.identity,report.identity_end))) throw Error('incompatible_reports');
    for (const key of ['model','addon_sha256','machine','configuration','lock_sha256','package_lock_sha256','revision','worktree_sha256','harness']) {
      if (JSON.stringify(report.identity[key])!==JSON.stringify(first[key])) throw Error('incompatible_reports');
    }
    if (report.kind==='quality' && (!report.selection.selected || report.rows.length>report.selection.selected ||
      report.status==='completed' && report.rows.length!==report.selection.selected)) throw Error('incomplete_quality');
    if (report.kind==='e2e' && (!report.correctness.length || !report.adapters.length || !report.host.length)) throw Error('incomplete_e2e');
    if (report.kind==='performance' && (!report.expected_measurements || report.status==='completed' && report.rows.length!==report.expected_measurements || report.stage!=='screen' || report.qualification!=='not_qualified')) throw Error('incomplete_performance');
  }
}
export function render(snapshot) {
  const {quality:q,e2e:e,performance:p} = snapshot;
  const model = Object.hasOwn(snapshot.evidence.quality.identity.model,'head.safetensors') ? 'local BERT candidate' : 'pinned `mudes/en-base`';
  const lines = ['## Battle-tested, with receipts','',
    'Craig’s replacement engine is checked for exact preservation outside selected spans. Detection is measured separately against public reference labels.', '',
    '| Check | Observed result |','| --- | --- |',
    `| Known-span replacement and abstention | ${e.correctness.passed}/${e.correctness.total} checks passed |`,
    `| Rust and Node CLI end-to-end checks | ${e.adapters.passed}/${e.adapters.total} checks passed |`,
    `| Installed Claude Code delivery and recovery | ${e.host.requested?`${e.host.passed}/${e.host.requested} requested checks passed`:'not requested'} |`];
  for (const [track,data] of Object.entries(q.tracks)) {
    const m = data.upstream_classification;
    lines.push(`| ${track==='profanity'?'Scunthorpe flag':'HateCheck hate-label'} agreement | ${percent(m.accuracy)} on ${m.scored}/${m.examples} sampled cases; F1 ${m.f1===null?'unavailable':m.f1.toFixed(3)}; ${m.abstentions??0} abstentions |`);
  }
  for (const [strategy,r] of Object.entries(q.replacement)) lines.push(`| Unmatched text: \`${strategy}\` | ${r.preservation_passed}/${r.successful} successful outputs preserved; ${r.abstentions} abstentions |`);
  lines.push('',`Measured ${escape(snapshot.date.slice(0,10))} UTC on ${escape(snapshot.machine.cpu)} (${escape(snapshot.machine.platform)}, ${escape(snapshot.machine.arch)}), CPU FP32, ${model}.`,
    'The external screen samples English cases by category and label; it is not a population accuracy estimate. HateCheck labels hate speech, which differs from Craig’s offensive-span policy.',
    q.reviewed_span_examples ? `Reviewed span scoring covers ${q.reviewed_span_examples} strategy-case observations; see the evidence for denominators.` : 'Reviewed external replacement spans are unavailable. These classification figures do not establish successful removal of targeted abuse.', '');
  if(p.protocol!==2) lines.push('Historical pilot: native timing included a separate profiling inference before reporting. Worker timeouts may have discarded completed measurements. This pilot cannot resolve those failures.','');
  lines.push('| Resident rewrite fixture | Native maximum | Node-API maximum | Failed combinations |','| --- | ---: | ---: | ---: |');
  for(const id of new Set(p.rows.map(r=>r.id))) {
    const rows=p.rows.filter(r=>r.id===id);
    const maximum=boundary=>{const values=rows.filter(r=>r.boundary===boundary && r.status==='completed').map(r=>r.summary?.max_ms).filter(Number.isFinite);return values.length?Math.max(...values):null;};
    lines.push(`| ${escape(id)} | ${ms(maximum('native'))} | ${ms(maximum('node_api'))} | ${rows.filter(r=>r.status!=='completed').length}/${rows.length} |`);
  }
  lines.push('',`Performance coverage: ${p.completed}/${p.expected} measurements completed successfully; ${p.failed} failed. Maxima above combine strategies only within the same fixture; failed timings are not success samples.`,
    `Recorded configuration: ${p.config.runs} run(s), ${p.config.samples} measured sample(s) and ${p.config.warmup} warmup(s) per fixture, strategy, and boundary; ${p.config.timeout_ms/1000} s worker deadline.`,
    `This is a screen, not latency qualification. The all-input maximum-below-500-ms screen ${p.under_500_ms?'passed':'did not pass'}. Cold load, process startup, per-fixture timing, and peak RSS are recorded separately in the evidence.`,
    p.protocol===2?'Latency runs exclude stage profiling. Matched per-fixture strategy comparisons and explicit failure causes are retained in the snapshot.':'No tail-latency or strategy-speed claim is supported by this pilot.', '',
    '[Evidence snapshot](benchmarks/results/latest.json) · [Reproduce the checks](docs/benchmarking.md)', '');
  return lines.join('\n');
}
async function snapshotFromReports(directory) {
  const reports = await Promise.all(['quality','e2e','performance'].map(k=>json(resolve(directory,`${k}.json`))));
  validateReports(reports);
  const [q,e,p] = reports;
  const tracks = Object.fromEntries(Object.entries(q.tracks).map(([track,data])=>[track,{...data,...replacementSummary(q.rows.filter(r=>r.track===track))}]));
  const snapshot = {version:1,date:new Date().toISOString(),machine:q.identity.machine,
    evidence:Object.fromEntries(await Promise.all(reports.map(async r=>[r.kind,{sha256:digest(await readFile(resolve(directory,`${r.kind}.json`))),identity:r.identity,status:r.status}]))),
    quality:{selection:q.selection,corpus_sha256:q.corpus_sha256,annotations_sha256:q.annotations_sha256,annotation_policy_versions:q.annotation_policy_versions??null,tracks,replacement:q.replacement,
      reviewed_spans:q.reviewed_spans,reviewed_policies:q.reviewed_policies??null,reviewed_span_examples:Object.values(q.reviewed_spans).reduce((n,r)=>n+r.examples,0)},
    e2e:{correctness:{passed:e.correctness.filter(r=>r.passed).length,total:e.correctness.length},
      adapters:{passed:e.adapters.filter(r=>r.passed).length,total:e.adapters.length},
      host:{passed:e.host.filter(r=>r.status==='passed').length,requested:e.host.filter(r=>r.status!=='not_requested').length,results:e.host},
      failures:[...e.correctness,...e.adapters].filter(r=>!r.passed),diagnostics:e.diagnostics??[]},
    performance:{protocol:p.protocol??1,config:p.config,identity_stable:p.identity_stable??null,filtered:p.filtered??false,expected:p.expected_measurements,completed:p.rows.filter(r=>r.status==='completed').length,failed:p.rows.filter(r=>r.status!=='completed').length,
      under_500_ms:p.under_500_ms,distributions:p.distributions,matched_comparisons:p.matched_comparisons??[],rows:p.rows.map(({times_ms,...row})=>row)}};
  return snapshot;
}

export async function report(directory) {
  const snapshot = await snapshotFromReports(directory);
  await save(resolve(directory,'snapshot.json'),snapshot);
  await writeFile(resolve(directory,'report.md'),render(snapshot),{flag:'wx'});
  return snapshot;
}

export async function updateReadme(directory) {
  const snapshot = await json(resolve(directory,'snapshot.json'));
  const reports = await Promise.all(['quality','e2e','performance'].map(k=>json(resolve(directory,`${k}.json`))));
  validateReports(reports);
  if(reports.some(r=>r.identity_stable===false) || reports.find(r=>r.kind==='performance').filtered) throw Error('incompatible_reports');
  for(const r of reports) {
    if(snapshot.evidence[r.kind].sha256!==digest(await readFile(resolve(directory,`${r.kind}.json`)))) throw Error('snapshot_evidence_mismatch');
  }
  const recomputed = await snapshotFromReports(directory);
  if(JSON.stringify({...recomputed,date:snapshot.date})!==JSON.stringify(snapshot)) throw Error('snapshot_summary_mismatch');
  const readme = await readFile('README.md','utf8');
  const start = '<!-- benchmark-results:start -->', end = '<!-- benchmark-results:end -->';
  if(readme.split(start).length!==2 || readme.split(end).length!==2 || readme.indexOf(start)>readme.indexOf(end)) throw Error('missing_readme_markers');
  const rendered = render(snapshot);
  await writeFile('benchmarks/results/latest.json',JSON.stringify(snapshot,null,2)+'\n');
  await writeFile('README.md',readme.slice(0,readme.indexOf(start)+start.length)+'\n'+rendered+readme.slice(readme.indexOf(end)));
}
