import {readFile,writeFile} from 'node:fs/promises';
import {resolve} from 'node:path';
import {pathToFileURL} from 'node:url';

export function qualityGate(regressions, candidate, baseline) {
  const reasons=[];
  const reports=[...regressions,candidate,baseline];
  if(!regressions.length||reports.some(r=>!r.examples||r.errors||r.results?.length!==r.examples)) reasons.push('incomplete_quality_evidence');
  if(reports.some(r=>!r.labels_reviewed||r.results.some(x=>!x.reviewed))) reasons.push('labels_require_review');
  if(regressions.some(r=>r.exact_rewrite_matches!==r.examples)) reasons.push('regression_mismatch');
  if(regressions.some(r=>r.model_sha256!==candidate.model_sha256)) reasons.push('candidate_artifact_mismatch');
  if(candidate.corpus_sha256!==baseline.corpus_sha256) reasons.push('evaluation_corpus_mismatch');
  if(candidate.results.some(r=>r.expected_outcome==='unchanged'&&r.actual_outcome!=='unchanged')) reasons.push('clean_case_changed');
  if(candidate.results.some(r=>r.expected_outcome==='protected_span_detected'&&r.actual_outcome!==r.expected_outcome)) reasons.push('protected_case_missed');
  if([candidate,baseline].some(r=>!r.normalized_characters||['precision','recall'].some(k=>!Number.isFinite(r.normalized_characters[k])||r.normalized_characters[k]<0||r.normalized_characters[k]>1))) reasons.push('missing_normalized_metrics');
  else for(const metric of ['precision','recall']) if(candidate.normalized_characters[metric]<baseline.normalized_characters[metric]) reasons.push(`${metric}_below_reference`);
  return {version:1,status:reasons.length?'unmet':'quality_pass',model_sha256:candidate.model_sha256,corpus_sha256:candidate.corpus_sha256,reasons};
}
async function main(args) {
  const o={};for(let i=0;i<args.length;i+=2){if(!['--regressions','--candidate','--baseline','--output'].includes(args[i])||!args[i+1])throw Error('invalid_arguments');o[args[i].slice(2)]=args[i+1];}
  if(Object.keys(o).length!==4)throw Error('missing_arguments');
  const read=async p=>JSON.parse(await readFile(p,'utf8'));
  const report=qualityGate(await Promise.all(o.regressions.split(',').map(read)),await read(o.candidate),await read(o.baseline));
  await writeFile(o.output,JSON.stringify(report,null,2)+'\n',{flag:'wx',mode:0o600});
  console.log(JSON.stringify(report));if(report.status!=='quality_pass')process.exitCode=1;
}
if(process.argv[1]&&import.meta.url===pathToFileURL(resolve(process.argv[1])).href)main(process.argv.slice(2)).catch(()=>{console.error('quality_gate_failed');process.exitCode=1;});
