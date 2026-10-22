import {createRequire} from 'node:module';
import {readFile, appendFile, mkdir} from 'node:fs/promises';
import {appendFileSync} from 'node:fs';
import {dirname, resolve} from 'node:path';
import os from 'node:os';
import {json, save, digest} from './corpus.mjs';
import {child, identity, sameIdentity, errorCode} from './common.mjs';
import {fixtureText, validateResults} from '../benchmark-inference.mjs';

const successes = ['rewritten','unchanged','protected_span_detected'];
const outcomes = [...successes,'invalid_input','invalid_options','invalid_span','model_unavailable','model_incompatible',
  'inference_failed','processing_timeout','busy','policy_unavailable','benchmark_execution_failed'];
const finite = n => Number.isFinite(n) && n >= 0;
const emit = value => process.stdout.write(JSON.stringify(value)+'\n');
export const environmentSample = () => ({recorded_at:new Date().toISOString(),logical_cpus:os.cpus().length,load_average:os.loadavg()});

export async function ffiWorker(request) {
  const binding = createRequire(import.meta.url)('../../native/index.cjs');
  const started = performance.now();
  const detector = await binding.loadDetector({modelDir:request.model,policy:'offensive_spans',threshold:0.5,backend:'cpu'});
  const load_ms = performance.now()-started, results = [];
  if (request.stream) emit({event:'loaded',load_ms});
  for (const fixture of request.fixtures) {
    const row = {id:fixture.id,bytes:Buffer.byteLength(fixture.text),times_ms:[],outcomes:{},errors:{}};
    for (let n=0;n<request.samples+request.warmup;n++) {
      const start = performance.now(); let status;
      try { const result=await detector.rewrite(fixture.text,{strategy:request.replacement_strategy}); status=result.replacementCount?'rewritten':'unchanged'; }
      catch(error) { status=errorCode(error); }
      const elapsed = performance.now()-start;
      if(n<request.warmup) { if(request.stream) emit({event:'warmup',id:fixture.id}); continue; }
      row.times_ms.push(elapsed); row.outcomes[status]=(row.outcomes[status]??0)+1;
      if(!successes.includes(status)) row.errors[status]=(row.errors[status]??0)+1;
      if(request.stream) emit({event:'sample',id:fixture.id,index:n-request.warmup,elapsed_ms:elapsed,outcome:status});
    }
    results.push(row);
  }
  return {replacement_strategy:request.replacement_strategy,load_ms,peak_rss_bytes:process.resourceUsage().maxRSS*1024,results};
}

function safeCounts(counts) {
  if (!counts || typeof counts!=='object' || Array.isArray(counts) || Object.entries(counts).some(([key,n])=>!outcomes.includes(key)||!Number.isSafeInteger(n)||n<1)) throw Error('schema_mismatch');
  return Object.fromEntries(Object.entries(counts));
}
export function decodeMeasurement(stdout, fixture, samples, strategy) {
  const evidence = validateResults(JSON.parse(stdout),[fixture],samples);
  if(evidence.replacement_strategy!==strategy) throw Error('strategy_mismatch');
  if(!finite(evidence.load_ms) || !finite(evidence.peak_rss_bytes)) throw Error('schema_mismatch');
  const measured = evidence.results[0];
  const errors=safeCounts(measured.errors), counts=safeCounts(measured.outcomes);
  if(Object.entries(counts).some(([key,n])=>!successes.includes(key) && errors[key]!==n) ||
    Object.entries(errors).some(([key,n])=>successes.includes(key)||counts[key]!==n)) throw Error('schema_mismatch');
  const failed = Object.keys(errors).length>0;
  return {status:failed?'failed':'completed',load_ms:evidence.load_ms,peak_rss_bytes:evidence.peak_rss_bytes,
    times_ms:measured.times_ms,outcomes:counts,errors,summary:measured.summary,completed_samples:measured.times_ms.length,
    failure_phase:failed?'rewrite':null,cause:failed?(errors.processing_timeout?'processing_timeout':'inference_error'):null,
    ...(failed?{error:'runner_outcome_error'}:{}),
    rewrites_per_second:failed?null:1000*measured.times_ms.length/measured.times_ms.reduce((a,b)=>a+b,0)};
}

// Only validated numeric/categorical fields cross the worker-to-evidence boundary.
export function measurementStream(fixture, samples, warmup, checkpoint=()=>{}) {
  const state={phase:'load',load_ms:null,times_ms:[],outcomes:{},errors:{},warmups:0,final:null};
  return {state,line(line) {
    let event;
    try { event=JSON.parse(line); } catch { throw Error('malformed_json'); }
    if(state.final) throw Error('unexpected_event');
    let safe;
    if(event.event==='loaded' && state.phase==='load' && finite(event.load_ms)) {
      state.load_ms=event.load_ms; state.phase=warmup?'warmup':'rewrite';
      safe={event:'loaded',load_ms:event.load_ms};
    } else if(event.event==='warmup' && event.id===fixture.id && state.phase==='warmup' && state.warmups<warmup) {
      state.warmups++;
      if(state.warmups===warmup) state.phase='rewrite';
      safe={event:'warmup',completed:state.warmups};
    } else if(event.event==='sample' && state.phase==='rewrite' && event.id===fixture.id &&
      event.index===state.times_ms.length && event.index<samples && finite(event.elapsed_ms) && outcomes.includes(event.outcome)) {
      state.times_ms.push(event.elapsed_ms);
      state.outcomes[event.outcome]=(state.outcomes[event.outcome]??0)+1;
      if(!successes.includes(event.outcome)) state.errors[event.outcome]=(state.errors[event.outcome]??0)+1;
      safe={event:'sample',index:event.index,elapsed_ms:event.elapsed_ms,outcome:event.outcome};
    } else if(event.results && !event.event && state.phase==='rewrite') {
      state.final=line;
      return;
    } else throw Error('schema_mismatch');
    checkpoint(safe);
  }};
}

export function finishMeasurement(result, stream, fixture, samples, strategy) {
  const s=stream.state;
  const base={completed_samples:s.times_ms.length,expected_samples:samples,censored:true,failure_phase:s.phase,
    load_ms:s.load_ms,times_ms:s.times_ms,outcomes:s.outcomes,errors:s.errors,rewrites_per_second:null,status:'failed',
    exit_code:result.status,signal:result.signal??null};
  if(result.error || result.status!==0) return {...base,cause:result.error==='child_timeout'?'worker_timeout':result.error??'process_exit',error:result.error??'runner_failed'};
  try {
    if(!s.final) throw Error(result.stdout.trim()?'truncated_report':'empty_report');
    const measured=decodeMeasurement(s.final,fixture,samples,strategy);
    if(JSON.stringify(measured.times_ms)!==JSON.stringify(s.times_ms)||Object.keys(measured.outcomes).length!==Object.keys(s.outcomes).length||
      Object.entries(measured.outcomes).some(([k,v])=>s.outcomes[k]!==v)) throw Error('stream_mismatch');
    return {...base,...measured,censored:false};
  } catch(error) {
    const cause=['truncated_report','empty_report','strategy_mismatch','stream_mismatch'].includes(error.message)?error.message:'schema_mismatch';
    return {...base,failure_phase:'report',cause,error:'invalid_runner_report'};
  }
}

export function matchedComparisons(rows) {
  const comparisons=[];
  for(const row of rows.filter(r=>r.strategy==='fixed_phrase')) {
    const other=rows.find(r=>r.run===row.run && r.id===row.id && r.boundary===row.boundary && r.strategy==='omit');
    if(!other) continue;
    const comparable=row.status==='completed' && other.status==='completed';
    comparisons.push({run:row.run,id:row.id,boundary:row.boundary,comparable,
      omit_to_fixed_p50_ratio:comparable?other.summary.p50_ms/row.summary.p50_ms:null});
  }
  return comparisons;
}

export async function performanceScreen(config) {
  const definitions = await json('tests/fixtures/performance.json');
  const runner = `target/${config.profile}/examples/inference${process.platform==='win32'?'.exe':''}`;
  const retry=config.retry?await json(config.retry):null;
  if(retry && (retry.kind!=='performance'||!Array.isArray(retry.rows))) throw Error('invalid_arguments');
  const selected=retry?.rows.filter(r=>r.status!=='completed');
  const key=r=>`${r.run}:${r.strategy}:${r.boundary}:${r.id}`;
  const tasks=[];
  for(let run=1;run<=config.runs;run++) for(const strategy of run%2?['fixed_phrase','omit']:['omit','fixed_phrase'])
    for(const definition of definitions) for(const boundary of ['native','node_api']) {
      const task={run,strategy,boundary,id:definition.id,definition};
      if(!selected || selected.some(r=>key(r)===key(task))) tasks.push(task);
    }
  if(!tasks.length || selected?.some(r=>!tasks.some(t=>key(t)===key(r)))) throw Error('invalid_arguments');
  const report = {version:1,protocol:2,kind:'performance',identity:await identity(config),fixture_sha256:digest(await readFile('tests/fixtures/performance.json')),
    runner_sha256:digest(await readFile(runner)),stage:'screen',qualification:'not_qualified',
    config:{samples:config.samples,warmup:config.warmup,runs:config.runs,timeout_ms:config['timeout-ms']},
    retry_sha256:retry?digest(await readFile(config.retry)):null,filtered:!!retry,
    load_average_start:os.loadavg(),environment_start:environmentSample(),expected_measurements:tasks.length,status:'running',rows:[]};
  await mkdir(dirname(config.output),{recursive:true});
  await save(`${config.output}.identity.json`,{...report,rows:undefined});
  for(const {definition,...task} of tasks) {
    const environment_start = environmentSample();
    const fixture={id:definition.id,text:fixtureText(definition)};
    process.stderr.write(`performance ${task.run}/${config.runs} ${task.strategy} ${task.boundary} ${fixture.id}\n`);
    const request={model:resolve(config.model),replacement_strategy:task.strategy,phase:'measure',stream:true,
      order:'fixture',detector:'bert',backend:'cpu',fp16:false,compute_units:'all',batch:1,samples:config.samples,warmup:config.warmup,fixtures:[fixture]};
    const stream=measurementStream(fixture,config.samples,config.warmup,event=>
      appendFileSync(`${config.output}.samples.jsonl`,JSON.stringify({...task,...event,environment:environmentSample()})+'\n',{mode:0o600}));
    const result=await child(task.boundary==='native'?runner:process.execPath,task.boundary==='native'?[]:['scripts/bench/run.mjs','ffi-worker'],
      {input:JSON.stringify(request),timeout:config['timeout-ms'],onLine:stream.line});
    const row={...task,environment_start,environment_end:environmentSample(),bytes:Buffer.byteLength(fixture.text),process_ms:result.milliseconds,...finishMeasurement(result,stream,fixture,config.samples,task.strategy)};
    report.rows.push(row);
    await appendFile(`${config.output}.progress.jsonl`,JSON.stringify(row)+'\n',{mode:0o600});
  }
  report.identity_end=await identity(config);
  report.identity_stable=sameIdentity(report.identity,report.identity_end) && report.runner_sha256===digest(await readFile(runner)) &&
    report.fixture_sha256===digest(await readFile('tests/fixtures/performance.json'));
  report.status=report.identity_stable && report.rows.every(r=>r.status==='completed')?'completed':'failed';
  report.load_average_end=os.loadavg();
  report.environment_end=environmentSample();
  report.maximum_observed_ms=Math.max(0,...report.rows.filter(r=>r.summary).map(r=>r.summary.max_ms));
  report.under_500_ms=!report.filtered && report.status==='completed' && report.maximum_observed_ms<500;
  report.distributions={};
  report.matched_comparisons=matchedComparisons(report.rows);
  await save(config.output,report);
  return report;
}

export async function profileScreen(config) {
  const runner=`target/${config.profile}/examples/inference${process.platform==='win32'?'.exe':''}`;
  const report={version:1,kind:'profiling',identity:await identity(config),runner_sha256:digest(await readFile(runner)),
    fixture_sha256:digest(await readFile('tests/fixtures/performance.json')),environment_start:environmentSample(),status:'running',rows:[]};
  await mkdir(dirname(config.output),{recursive:true});
  for(const definition of await json('tests/fixtures/performance.json')) {
    const environment_start = environmentSample();
    const fixture={id:definition.id,text:fixtureText(definition)};
    process.stderr.write(`profile ${fixture.id}\n`);
    const result=await child(runner,[],{timeout:config['timeout-ms'],input:JSON.stringify({model:resolve(config.model),phase:'profile',
      backend:'cpu',batch:1,samples:1,warmup:0,fixtures:[fixture]})});
    const row={id:fixture.id,environment_start,environment_end:environmentSample(),bytes:Buffer.byteLength(fixture.text),status:'failed',failure_phase:'profile',process_ms:result.milliseconds};
    try {
      if(result.error||result.status!==0) throw Error(result.error??'process_exit');
      const evidence=JSON.parse(result.stdout), measured=evidence.results?.[0];
      if(measured?.id!==fixture.id || !finite(evidence.load_ms) || !finite(evidence.peak_rss_bytes)) throw Error('schema_mismatch');
      row.load_ms=evidence.load_ms; row.peak_rss_bytes=evidence.peak_rss_bytes;
      const fields=['tokenize_ms','tokens','windows','completed_windows','encoder_ms','classifier_ms','offsets_ms'];
      if(fields.some(k=>!finite(measured.stages?.[k]))) throw Error('schema_mismatch');
      row.stages=Object.fromEntries(fields.map(k=>[k,measured.stages[k]]));
      if(measured.profile_error) throw Error(outcomes.includes(measured.profile_error)?measured.profile_error:'profile_error');
      if(!finite(measured.replacement_only_ms)) throw Error('schema_mismatch');
      row.replacement_only_ms=measured.replacement_only_ms; row.status='completed'; row.failure_phase=null;
    } catch(error) { row.cause=['child_timeout','process_exit','processing_timeout','profile_error'].includes(error.message)?error.message:'invalid_profile_report'; }
    report.rows.push(row);
    await appendFile(`${config.output}.progress.jsonl`,JSON.stringify(row)+'\n',{mode:0o600});
  }
  report.identity_end=await identity(config);
  report.identity_stable=sameIdentity(report.identity,report.identity_end) && report.runner_sha256===digest(await readFile(runner)) &&
    report.fixture_sha256===digest(await readFile('tests/fixtures/performance.json'));
  report.status=report.identity_stable && report.rows.every(r=>r.status==='completed')?'completed':'failed';
  report.environment_end=environmentSample();
  await save(config.output,report);
  return report;
}
