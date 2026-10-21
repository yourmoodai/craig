import {readFile, mkdir} from 'node:fs/promises';
import {resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import {json, save, digest, validate, applyAnnotations, annotationPolicies, annotationFor} from './corpus.mjs';

export function reserveGroups(rows, exposedIds) {
  const parent=new Map(rows.map(r=>[r.id,r.id]));
  const find=id=>{ let root=id; while(parent.get(root)!==root) root=parent.get(root);
    while(id!==root) { const next=parent.get(id); parent.set(id,root); id=next; } return root; };
  const groups=new Map();
  for(const row of rows) for(const key of [`text:${digest(row.text)}`,`family:${row.dataset}:${row.family}`]) {
    if(groups.has(key)) { const roots=[find(row.id),find(groups.get(key))].sort(); parent.set(roots[1],roots[0]); } else groups.set(key,row.id);
  }
  const exposed=new Set(exposedIds.map(find));
  return rows.map(row=>({id:row.id,group:digest(find(row.id)),partition:exposed.has(find(row.id))?'diagnostic':'reserved_evaluation'}));
}

export function performanceFailures(report, retry) {
  const key=r=>`${r.run}:${r.strategy}:${r.boundary}:${r.id}`;
  return report.rows.filter(r=>r.status!=='completed').map(row=>{
    const observed=retry?.rows.find(r=>key(r)===key(row));
    const known=['worker_timeout','processing_timeout','empty_report','truncated_report','schema_mismatch','invalid_stream','process_exit'];
    let classification='unresolved', certainty='unresolved';
    if(observed?.cause==='processing_timeout') { classification='runtime_deadline'; certainty='reproduced'; }
    else if(observed?.status==='completed' && row.error==='child_timeout' && row.boundary==='native') {
      classification='profiling_coupled_watchdog'; certainty='consistent_with_harness_repair';
    } else if(observed?.status==='completed') { classification='not_reproduced'; certainty='unresolved'; }
    else if(known.includes(observed?.cause)) classification=observed.cause;
    return {run:row.run,id:row.id,strategy:row.strategy,boundary:row.boundary,classification,certainty,
      original_code:['child_timeout','runner_outcome_error','invalid_runner_report'].includes(row.error)?row.error:'unclassified',
      retry_status:observed?.status??'not_run',retry_cause:known.includes(observed?.cause)?observed.cause:null};
  });
}

export function qualityInventory(corpus, quality) {
  const byId=new Map(corpus.map(r=>[r.id,r])), seen=new Set();
  return quality.rows.map(result=>{
    const row=byId.get(result.id);
    if(!row || seen.has(row.id) || row.upstream_label!==result.expected || row.dataset!==result.dataset || row.category!==result.category) throw Error('triage_identity_mismatch');
    seen.add(row.id);
    const abstention=result.detection_status==='protected_span_detected';
    const mismatch=typeof result.predicted==='boolean' && result.expected!==result.predicted;
    const reason=abstention?'protected_abstention':result.predicted===null?'execution_error':mismatch?(result.expected?'upstream_false_negative':'upstream_false_positive'):'upstream_agreement';
    return {id:row.id,text_sha256:digest(row.text),dataset:row.dataset,track:row.track,category:row.category,
      priority:abstention||mismatch||result.predicted===null?0:1,reason,
      policies:Object.fromEntries(annotationPolicies.map(policy=>{
        const a=annotationFor(row,policy);
        return [policy,{classification:a?.reviewed?a.classification??'unresolved':'unresolved',review_status:a?.reviewed?'reviewed':'unresolved'}];
      }))};
  }).sort((a,b)=>a.priority-b.priority||a.id.localeCompare(b.id));
}

export async function triage(config) {
  const corpusBytes=await readFile(config.corpus), corpus=JSON.parse(corpusBytes);
  validate(corpus.rows);
  const q=await json(config.quality), p=await json(config.performance);
  if(q.kind!=='quality' || p.kind!=='performance' || q.corpus_sha256!==digest(corpusBytes)) throw Error('triage_identity_mismatch');
  if(config.annotations) applyAnnotations(corpus.rows,await json(config.annotations));
  const retry=config.retry?await json(config.retry):null;
  if(retry && (retry.kind!=='performance' || retry.retry_sha256!==digest(await readFile(config.performance)))) throw Error('triage_identity_mismatch');
  const inventory=qualityInventory(corpus.rows,q);
  const partitions=reserveGroups(corpus.rows,inventory.map(r=>r.id));
  const counts=Object.fromEntries(['scunthorpe','hatecheck'].map(dataset=>[dataset,{
    diagnostic:partitions.filter(r=>r.partition==='diagnostic'&&r.id.startsWith(dataset+'-')).length,
    reserved_evaluation:partitions.filter(r=>r.partition==='reserved_evaluation'&&r.id.startsWith(dataset+'-')).length,
    reserved_english:partitions.filter(r=>r.partition==='reserved_evaluation'&&r.id.startsWith(dataset+'-')&&corpus.rows.find(c=>c.id===r.id).language==='en').length
  }]));
  const overlay=inventory.flatMap(row=>annotationPolicies.map(policy=>({id:row.id,text_sha256:row.text_sha256,
    annotation:annotationFor(corpus.rows.find(r=>r.id===row.id),policy)??{unit:'utf16',spans:[],origin:'manual',policy,policy_version:'1',
      reviewed:false,review_status:'unresolved',classification:'unresolved'}})));
  const report={version:1,kind:'triage',status:'pending_review',
    evidence:{corpus_sha256:digest(corpusBytes),quality_sha256:digest(await readFile(config.quality)),performance_sha256:digest(await readFile(config.performance)),
      retry_sha256:config.retry?digest(await readFile(config.retry)):null,annotations_sha256:config.annotations?digest(await readFile(config.annotations)):null},
    summary:{selected:inventory.length,priority_review:inventory.filter(r=>r.priority===0).length,
      reviewed_policy_cases:inventory.reduce((n,r)=>n+Object.values(r.policies).filter(p=>p.review_status==='reviewed').length,0),partitions:counts},
    performance:performanceFailures(p,retry),quality:inventory};
  await mkdir(config.output,{recursive:false});
  await save(resolve(config.output,'triage.json'),report);
  await save(resolve(config.output,'annotations.json'),overlay);
  await save(resolve(config.output,'partitions.json'),{version:1,corpus_sha256:digest(corpusBytes),method:'connected_exact_text_and_upstream_family',rows:partitions});
  return report;
}

if(process.argv[1] && import.meta.url===pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const args=process.argv.slice(2);
    if(args.includes('--help')) console.log('usage: npm run bench:triage -- --quality FILE --performance FILE --output NEW_DIR [--corpus FILE] [--retry FILE] [--annotations FILE]\nExports content-free triage, unresolved review overlays, and conservative duplicate/family-separated partitions. No labels are approved automatically.');
    else {
      const config={quality:undefined,performance:undefined,output:undefined,corpus:'.cache/bench-corpus/corpus.json',retry:undefined,annotations:undefined};
      for(let i=0;i<args.length;i+=2) {
        const key=args[i].replace(/^--/,'');
        if(!Object.hasOwn(config,key)||!args[i+1]) throw Error('invalid_arguments');
        config[key]=args[i+1];
      }
      if(!config.quality||!config.performance||!config.output) throw Error('invalid_arguments');
      console.log(JSON.stringify((await triage(config)).summary));
    }
  } catch(error) { console.error(['invalid_arguments','triage_identity_mismatch','annotation_conflict'].includes(error.message)?error.message:'triage_failed'); process.exitCode=1; }
}
