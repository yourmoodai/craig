import {spawn} from 'node:child_process';
import {createHash} from 'node:crypto';
import {readFile,writeFile,mkdir,access} from 'node:fs/promises';
import {resolve} from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {summarize,passesLatency} from './benchmark-inference.mjs';

export function compareReports(reports, requiredIds=[]) {
  if(!reports.length)throw Error('missing_reports');
  const first=reports[0].report;
  const keys=['fixture_sha256','harness_sha256','target_ms','boundary','order','node','rustc','lock_sha256'];
  for(const {report:r} of reports){
    if(r.runs.some(run=>run.allocation_instrumented))throw Error('instrumented_timing');
    if(JSON.stringify(r.threads)!==JSON.stringify(first.threads)||keys.some(k=>r[k]!==first[k])||['cpu','os','arch','memory_bytes'].some(k=>r.machine[k]!==first.machine[k]))throw Error('incompatible_reports');
  }
  const names=[...new Set(reports.map(r=>r.name))];const candidates=[];
  for(const name of names){
    const selected=reports.filter(r=>r.name===name);const initial=selected[0].report;
    for(const {report:r} of selected){if(['model_sha256','runner_sha256','addon_sha256','threshold','backend','precision','window_batch','compute_units','coreml_export_sha256'].some(k=>r[k]!==initial[k])||JSON.stringify(r.artifacts_sha256)!==JSON.stringify(initial.artifacts_sha256))throw Error('candidate_artifact_mismatch');}
    const boundaries={};
    for(const boundary of ['runs','ffi_runs']){
      const runs=selected.flatMap(r=>r.report[boundary]);const ids=first.runs[0]?.results.map(r=>r.id)??[];
      if(!ids.length||runs.length!==selected.length||runs.some(r=>r.results.length!==ids.length||r.results.some((row,i)=>row.id!==ids[i])))throw Error('incomplete_results');
      boundaries[boundary]=ids.map((id,i)=>{const rows=runs.map(r=>r.results[i]);if(rows.some(r=>r.bytes!==rows[0].bytes||!r.times_ms.length||Object.values(r.outcomes).some(n=>!Number.isInteger(n)||n<0)||Object.values(r.outcomes).reduce((a,b)=>a+b,0)!==r.times_ms.length))throw Error('incomplete_samples');const errors=rows.some(r=>r.profile_error||Object.keys(r.errors).length);return {id,...summarize(rows.flatMap(r=>r.times_ms),first.target_ms),errors,passes:!errors&&rows.every(r=>passesLatency([{...r,summary:summarize(r.times_ms,first.target_ms)}],first.target_ms))};});
    }
    const covered=requiredIds.length>0&&boundaries.runs.length===requiredIds.length&&boundaries.runs.every((row,i)=>row.id===requiredIds[i])&&selected.every(r=>!r.report.qualification_reasons?.includes('filtered_fixtures'));
    const complete=covered&&selected.length===3&&selected.every(r=>r.report.warmup>=20&&[...r.report.runs,...r.report.ffi_runs].every(run=>run.results.every(row=>row.times_ms.length>=10000)));
    const pass=Object.values(boundaries).flat().every(r=>r.passes)&&selected.every(r=>!r.report.errors.length);
    candidates.push({name,evidence:Object.fromEntries(['runner_sha256','addon_sha256','fixture_sha256','harness_sha256','lock_sha256','threshold','backend','precision','window_batch','compute_units','artifacts_sha256','coreml_export_sha256'].map(k=>[k,initial[k]])),model_sha256:initial.model_sha256,precision:initial.precision,arithmetic:initial.detector==='bert'?initial.precision:'float64_parameters_and_marginals_float32_spans',parity_status:initial.detector==='bert'&&initial.precision==='float16'?'previously_failed':initial.detector==='bert'?'requires_separate_validation':'not_applicable_changed_detector',quality_status:'provisional_label_agreement',latency_status:pass?(complete?'latency_qualified':'screen_pass'):'unmet',boundaries});
  }
  for(const candidate of candidates){candidate.speedups={};for(const baseline of candidates.filter(c=>c.name.startsWith('bert_'))){candidate.speedups[baseline.name]={};for(const boundary of ['runs','ffi_runs']){candidate.speedups[baseline.name][boundary]=candidate.boundaries[boundary].map((row,i)=>{const base=baseline.boundaries[boundary][i];return {id:row.id,median_ratio:row.errors||base.errors?null:base.p50_ms/row.p50_ms,observed_max_ratio:row.errors||base.errors?null:base.max_ms/row.max_ms};});}}}
  return {version:1,target_ms:first.target_ms,boundary:first.boundary,fixture_sha256:first.fixture_sha256,machine:first.machine,quality_status:'provisional_label_agreement',candidates};
}
function run(cmd,args){return new Promise((resolvePromise,reject)=>{const child=spawn(cmd,args,{stdio:['ignore','ignore','pipe']});let stderr='';child.stderr.on('data',b=>{stderr=(stderr+b).slice(-8192);});child.on('error',()=>reject(Error('child_start_failed')));child.on('exit',(code,signal)=>code===0?resolvePromise():reject(Error(`child_failed:${code??signal}`)));});}
export async function main(argv){
  if(argv.includes('--help')){console.log('usage: npm run bench:decisions -- --models DIR --output NEW_DIR [--stage pilot|screen|qualify] [--candidates lexical,crf,bert_cpu,bert_coreml_fp32,bert_coreml_fp16]\nbuild: npm run build && npm run build:decisions\ntrain: npm run decisions -- train --corpus tests/fixtures/corpus.jsonl --output NEW_MODEL_DIR\npilot: 1 run x 3 samples, 2 warmups; screen: 3 x 100, 20 warmups; qualify: 3 x 10000, 20 warmups. Qualification requires --screen-report PATH from a passing screen and --quality-report PATH for promotion. Unmet qualification exits nonzero after writing evidence. Pilot watchdog: 600000 ms per native/FFI process; override with --timeout-ms. All stages report peak RSS. Models are resident; runtime is offline.');return;}
  const options={stage:'screen',candidates:'lexical,crf,bert_cpu,bert_coreml_fp32,bert_coreml_fp16',addon:'native/craig.darwin-arm64.node',runner:'target/profiling/examples/inference','coreml-addon':'.cache/native-coreml/craig.darwin-arm64.node','coreml-runner':'target/coreml/profiling/examples/inference','decision-runner':'target/profiling/decision','bert-model':'.cache/models/mudes-en-base','coreml-fp32':'.cache/coreml-f32','coreml-fp16':'.cache/coreml-f16-finite','timeout-ms':null,'screen-report':null,'quality-report':null};
  for(let i=0;i<argv.length;i+=2){const key=argv[i]?.replace(/^--/,'');if(![...Object.keys(options),'models','output'].includes(key)||argv[i+1]===undefined)throw Error('invalid_arguments');options[key]=argv[i+1];}
  const profile={pilot:[1,3,2],screen:[3,100,20],qualify:[3,10000,20]}[options.stage];
  const names=options.candidates.split(',');if(!profile||!options.models||!options.output||new Set(names).size!==names.length||names.some(n=>!['lexical','crf','bert_cpu','bert_coreml_fp32','bert_coreml_fp16'].includes(n)))throw Error('invalid_arguments');
  const candidates=names.map(name=>{const coreml=name.includes('coreml');const kind=name.startsWith('bert_')?'bert':name;return {name,kind,coreml,model:kind==='bert'?options['bert-model']:resolve(options.models,kind),runner:coreml?options['coreml-runner']:options.runner,addon:coreml?options['coreml-addon']:options.addon,precision:name.endsWith('fp16')?'float16':'float32'};});
  for(const c of candidates){await access(c.runner);await access(c.addon);await access(resolve(c.model,c.kind==='bert'?'pytorch_model.bin':'decision.json'));}
  await access(options['decision-runner']);
  const timeout=options['timeout-ms']===null?(options.stage==='pilot'?600000:120000+20*(profile[1]+profile[2]+1)*1000):Number(options['timeout-ms']);
  if(!Number.isInteger(timeout)||timeout<1||timeout>2147483647)throw Error('invalid_timeout');
  if(options.stage==='qualify'){
    if(!options['screen-report'])throw Error('qualification_requires_screen_report');
    const screen=JSON.parse(await readFile(options['screen-report'],'utf8'));
    if(screen.stage!=='screen'||screen.target_ms!==500||candidates.some(c=>!screen.candidates.some(r=>r.name===c.name&&r.latency_status==='screen_pass')))throw Error('screening_not_passed');
    for(const c of candidates){const hash=createHash('sha256').update(await readFile(resolve(c.model,c.kind==='bert'?'pytorch_model.bin':'decision.json'))).digest('hex');if(!screen.candidates.some(r=>r.name===c.name&&r.model_sha256===hash))throw Error('screen_artifact_mismatch');}
  }
  const screenEvidence=options.stage==='qualify'?JSON.parse(await readFile(options['screen-report'],'utf8')):null;
  await mkdir(options.output);const reports=[];const failures=[];
  for(let round=0;round<profile[0];round++){
    const order=[...candidates.slice(round%candidates.length),...candidates.slice(0,round%candidates.length)];
    for(const c of order){if(failures.some(f=>f.name===c.name))continue;process.stderr.write(`benchmark ${c.name} run ${round+1}\n`);const output=resolve(options.output,`${c.name}-${round+1}.json`);
      const args=['scripts/benchmark-inference.mjs','--detector',c.kind,'--model',c.model,'--runner',c.runner,'--addon',c.addon,'--backend',c.coreml?'core_ml':'cpu','--precision',c.precision,'--samples',String(profile[1]),'--warmup',String(profile[2]),'--runs','1','--target-ms','500','--fixtures-file','tests/fixtures/decision-performance.json','--order','interleaved','--timeout-ms',String(timeout),'--output',output];
      if(c.coreml)args.push('--coreml-dir',options[c.precision==='float16'?'coreml-fp16':'coreml-fp32']);
      try{await run(process.execPath,args);const result=JSON.parse(await readFile(output,'utf8'));if(screenEvidence){const prior=screenEvidence.candidates.find(x=>x.name===c.name);if(!prior.evidence||Object.entries(prior.evidence).some(([k,v])=>JSON.stringify(result[k])!==JSON.stringify(v)))throw Error('screen_artifact_mismatch');}reports.push({name:c.name,report:result});}catch{failures.push({name:c.name,run:round+1,code:'incomplete_benchmark',report:output});process.stderr.write(`benchmark incomplete ${c.name}\n`);}
    }
  }
  for(const c of candidates.filter(c=>!c.coreml&&!failures.some(f=>f.name===c.name))){for(const [id,corpus] of [['corpus','tests/fixtures/corpus.jsonl'],['independent','tests/fixtures/decision-heldout.jsonl']]){process.stderr.write(`evaluate ${c.name} ${id}\n`);await run(options['decision-runner'],['evaluate','--kind',c.kind,'--model',c.model,'--corpus',corpus,'--output',resolve(options.output,`${c.name}-quality-${id}.json`)]);}}
  const requiredIds=JSON.parse(await readFile('tests/fixtures/decision-performance.json','utf8')).map(r=>r.id);
  const valid=reports.filter(r=>!failures.some(f=>f.name===r.name));
  const report=valid.length?compareReports(valid,requiredIds):{version:1,target_ms:500,candidates:[]};report.failures=failures;report.comparison_harness_sha256=createHash('sha256').update(await readFile(fileURLToPath(import.meta.url))).digest('hex');report.stage=options.stage;report.samples_per_run=profile[1];report.warmup=profile[2];
  for(const c of report.candidates){const files=[];for(const id of ['corpus','independent']){const name=`${c.name}-quality-${id}.json`;try{const quality=JSON.parse(await readFile(resolve(options.output,name),'utf8'));files.push({file:name,...Object.fromEntries(['examples','characters','exact_span_matches','exact_span_eligible_examples','exact_rewrite_matches','labels_reviewed','errors'].map(k=>[k,quality[k]]))});}catch(e){if(e.code!=='ENOENT')throw e;}}c.quality=files;c.quality_status=files.length?'provisional_label_agreement':'not_evaluated_in_this_run';}
  if(options.stage==='qualify'){let quality;try{quality=JSON.parse(await readFile(options['quality-report'],'utf8'));}catch{}report.promotion_status=quality?.status==='quality_pass'&&failures.length===0&&report.candidates.length===candidates.length&&report.candidates.every(c=>c.latency_status==='latency_qualified'&&c.model_sha256===quality.model_sha256)?'eligible':'unmet';report.quality_gate=quality??{status:'missing'};if(report.promotion_status!=='eligible')process.exitCode=1;}
  await writeFile(resolve(options.output,'comparison.json'),JSON.stringify(report,null,2)+'\n',{flag:'wx',mode:0o600});
  const lines=['# Local decision detector comparison','',`Stage: ${options.stage}. Target: every complete rewrite below 500 ms. Quality labels remain provisional.`,'','| Candidate | Native maximum (ms) | Node-API maximum (ms) | Latency status |','| --- | ---: | ---: | --- |'];
  for(const c of report.candidates)lines.push(`| ${c.name} | ${Math.max(...c.boundaries.runs.map(r=>r.max_ms)).toFixed(2)} | ${Math.max(...c.boundaries.ffi_runs.map(r=>r.max_ms)).toFixed(2)} | ${c.latency_status} |`);
  for(const f of failures)lines.push(`| ${f.name} | — | — | incomplete; excluded from ratios |`);
  lines.push('','Per-fixture distributions, speedup ratios, errors, artifact identities, and provisional label metrics are in `comparison.json`. Screening is not qualification; latency qualification does not approve model quality. FP16 BERT previously failed numerical parity. Peak RSS excludes accelerator and daemon memory.');
  await writeFile(resolve(options.output,'comparison.md'),lines.join('\n')+'\n',{flag:'wx',mode:0o600});console.log(JSON.stringify({output:resolve(options.output),stage:options.stage,candidates:report.candidates.map(c=>({name:c.name,latency_status:c.latency_status}))}));
}
if(process.argv[1]&&import.meta.url===pathToFileURL(resolve(process.argv[1])).href)main(process.argv.slice(2)).catch(e=>{console.error(e.message);process.exitCode=1;});
