import {createHash} from 'node:crypto';
import {readFile, writeFile, access} from 'node:fs/promises';
import {createRequire} from 'node:module';
import {resolve} from 'node:path';
import {spawnSync} from 'node:child_process';
import os from 'node:os';
import {pathToFileURL,fileURLToPath} from 'node:url';

export function summarize(values, targetMs=500) {
  if (!Number.isFinite(targetMs)||targetMs<=0) throw Error('invalid_target');
  if (!values.length || values.some(v=>!Number.isFinite(v)||v<0)) throw Error('invalid_samples');
  const sorted=[...values].sort((a,b)=>a-b), n=sorted.length;
  const quantile=p=>sorted[Math.ceil(p*n)-1];
  // Order-statistic interval from exact binomial tails; unbounded tails remain explicit.
  const probabilities=new Array(n+1).fill(0);const mode=Math.floor((n+1)*0.99);
  probabilities[mode]=1;
  for(let k=mode;k>0;k--) probabilities[k-1]=probabilities[k]*k/(n-k+1)*0.01/0.99;
  for(let k=mode;k<n;k++) probabilities[k+1]=probabilities[k]*(n-k)/(k+1)*0.99/0.01;
  const total=probabilities.reduce((a,b)=>a+b,0);let cumulative=0,lower=0,upper=n+1;
  for(let k=0;k<=n;k++){cumulative+=probabilities[k]/total;if(cumulative<=0.025)lower=k+1;if(cumulative>=0.975){upper=k+1;break;}}
  return {samples:n,p50_ms:quantile(.5),p95_ms:quantile(.95),p99_ms:quantile(.99),max_ms:sorted.at(-1),exceedances:sorted.filter(v=>v>=targetMs).length,p99_interval_95_ms:[lower>0?sorted[lower-1]:0,upper<=n?sorted[upper-1]:null]};
}
export function passesLatency(rows, targetMs=500) {
  if(!Number.isFinite(targetMs)||targetMs<=0)throw Error('invalid_target');
  const allowed = new Set(['rewritten','unchanged','protected_span_detected']);
  return rows.length > 0 && rows.every(row => !row.profile_error && !Object.keys(row.errors).length &&
    Object.keys(row.outcomes).every(key => allowed.has(key)) && row.summary.max_ms < targetMs);
}
export function fixtureText({pattern,bytes,segments,seed=1}) {
  if(!Number.isInteger(bytes)||bytes<1||bytes>65536)throw Error('invalid_fixture');
  if(segments!==undefined){
    if(!Array.isArray(segments)||!segments.length||segments.some(s=>typeof s!=='string'||!s.length))throw Error('invalid_fixture');
    let state=seed>>>0;const pieces=[];let length=0;
    while(length<bytes){state=(Math.imul(state,1664525)+1013904223)>>>0;const piece=segments[state%segments.length].replaceAll('{n}',String(pieces.length));pieces.push(piece);length+=Buffer.byteLength(piece);}
    pattern=pieces.join('');
  }
  if(typeof pattern!=='string'||!pattern.length||!Number.isInteger(bytes)||bytes<1||bytes>65536)throw Error('invalid_fixture');
  const buffer=Buffer.from(pattern.repeat(Math.ceil(bytes/Buffer.byteLength(pattern))));
  let end=bytes;while(end>0 && (buffer[end]&0xc0)===0x80)end--;
  return buffer.subarray(0,end).toString('utf8')+' '.repeat(bytes-end);
}
function args(argv){const result={runner:'target/profiling/examples/inference',backend:'cpu',precision:'float32','compute-units':'all',batch:1,samples:20,warmup:3,runs:1,'timeout-ms':null,detector:'bert','target-ms':500,'fixtures-file':'tests/fixtures/performance.json',order:'fixture'};
  for(let i=0;i<argv.length;i+=2){const key=argv[i]?.replace(/^--/,'');if(!['runner','backend','precision','coreml-dir','compute-units','batch','samples','warmup','runs','output','fixture','addon','timeout-ms','detector','target-ms','fixtures-file','model','order'].includes(key)||argv[i+1]===undefined)throw Error('invalid_arguments');result[key]=argv[i+1];}
  if(result.backend==='core_ml'&&!argv.includes('--compute-units'))result['compute-units']='cpu_gpu';
  for(const key of ['batch','samples','warmup','runs','timeout-ms','target-ms']){if(key==='timeout-ms'&&result[key]===null)continue;result[key]=Number(result[key]);if(!Number.isInteger(result[key])||result[key]<(key==='warmup'?0:1))throw Error('invalid_arguments');}
  if(!['float32','float16'].includes(result.precision)||!result.output||!['cpu','metal','core_ml'].includes(result.backend)||![1,2,4,8].includes(result.batch)||result.samples>100000||result.runs>3||result.warmup>1000)throw Error('invalid_arguments');
  if(result['timeout-ms']!==null&&result['timeout-ms']>2147483647)throw Error('invalid_arguments');
  if(!['fixture','interleaved'].includes(result.order))throw Error('invalid_order');
  if(!['bert','lexical','crf'].includes(result.detector)||(result.detector!=='bert'&&(result.backend!=='cpu'||result.precision!=='float32'||result.batch!==1)))throw Error('invalid_detector_options');
  if(result.addon&&!result.addon.endsWith('.node'))throw Error('invalid_addon');
  if(!['all','cpu_only','cpu_gpu','cpu_neural_engine'].includes(result['compute-units'])||(result.backend==='core_ml'&&!result['coreml-dir']))throw Error('invalid_arguments');return result;
}
async function ffiRun(addon,{model,fixtures,samples,warmup,backend,precision,batch,coremlDir,computeUnits,detector:kind,targetMs,order}) {
      const binding=createRequire(import.meta.url)(resolve(addon));const start=performance.now();const detector=kind==='bert'?await binding.loadDetector({modelDir:model,threshold:0.5,policy:'offensive_spans',backend,precision,windowBatch:batch,coremlDir,computeUnits}):kind==='lexical'?await binding.loadDetector({detector:kind,modelDir:model}):await binding.loadExperimentalDetector({kind,artifactDir:model});const ffi={load_ms:performance.now()-start,results:[]};
      for(const fixture of fixtures) ffi.results.push({id:fixture.id,bytes:Buffer.byteLength(fixture.text),times_ms:[],errors:{},outcomes:{}});
      async function measure(i,record){const fixture=fixtures[i],row=ffi.results[i];const start=performance.now();let status;try{const r=await detector.rewrite(fixture.text);status=r.replacementCount?'rewritten':'unchanged';}catch(e){status=e.message;if(record&&status!=='protected_span_detected')row.errors[status]=(row.errors[status]??0)+1;}if(record){row.times_ms.push(performance.now()-start);row.outcomes[status]=(row.outcomes[status]??0)+1;}}
      if(order==='interleaved'){for(let n=0;n<warmup+samples;n++)for(let i=0;i<fixtures.length;i++)await measure(i,n>=warmup);}
      else {for(let i=0;i<fixtures.length;i++)for(let n=0;n<warmup+samples;n++)await measure(i,n>=warmup);}
      for(const row of ffi.results)row.summary=summarize(row.times_ms,targetMs);
      ffi.peak_rss_bytes=process.resourceUsage().maxRSS*1024;
      return ffi;

}
export function validateResults(result,fixtures,samples,targetMs=500) {
  if(!Array.isArray(result.results)||result.results.length!==fixtures.length)throw Error('incomplete_results');
  for(let i=0;i<fixtures.length;i++){
    const row=result.results[i];
    if(row.id!==fixtures[i].id||row.bytes!==Buffer.byteLength(fixtures[i].text)||!Array.isArray(row.times_ms)||row.times_ms.length!==samples||!row.errors||!row.outcomes||Object.values(row.outcomes).reduce((a,b)=>a+b,0)!==samples)throw Error('incompatible_results');
    row.summary=summarize(row.times_ms,targetMs);
  }
  return result;
}
export async function main(argv) {
  const options=args(argv);try{await access(options.output);throw Error('output_exists');}catch(error){if(error.code!=='ENOENT')throw error;}
  const fixtureBytes=await readFile(options['fixtures-file']);const all=JSON.parse(fixtureBytes);
  const requested=options.fixture?.split(',');if(requested?.some(id=>!all.some(f=>f.id===id)))throw Error('unknown_fixture');
  const fixtures=all.filter(f=>!requested||requested.includes(f.id)).map(f=>({id:f.id,text:fixtureText(f)}));
  options['timeout-ms']??=Math.min(2147483647,120000+fixtures.length*(options.samples+options.warmup+1)*1000);
  const model=options.model??process.env.MIDDLE_FINGER_MODEL_DIR??'.cache/models/mudes-en-base';
  const hash=b=>createHash('sha256').update(b).digest('hex');
  const report={version:3,detector:options.detector,boundary:'resident_rewrite',target_ms:options['target-ms'],target_statistic:'maximum',comparison:'strictly_less_than',threshold:0.5,policy:'offensive_spans',precision:options.precision,machine:{cpu:os.cpus()[0].model,logical_cpus:os.cpus().length,memory_bytes:os.totalmem(),os:os.release(),arch:os.arch(),load_average_start:os.loadavg()},threads:{rayon:process.env.RAYON_NUM_THREADS??'default',candle:process.env.CANDLE_NUM_THREADS??'default'},fixture_sha256:hash(fixtureBytes),model_sha256:hash(await readFile(resolve(model,options.detector==='bert'?'pytorch_model.bin':'decision.json'))),harness_sha256:hash(await readFile(fileURLToPath(import.meta.url))),runner_sha256:hash(await readFile(options.runner)),lock_sha256:hash(await readFile('Cargo.lock')),rustc:spawnSync('rustc',['--version'],{encoding:'utf8'}).stdout.trim(),node:process.version,runs:[],ffi_runs:[],errors:[],target_status:'unmet'};
  report.artifacts_sha256={};
  for(const name of ['config.json','model_args.json','tokenizer_config.json','vocab.txt','manifest.json','head.safetensors']){
    try{report.artifacts_sha256[name]=hash(await readFile(resolve(model,name)));}catch(error){if(error.code!=='ENOENT')throw error;}
  }
  report.warmup=options.warmup; report.runner_timeout_ms=options['timeout-ms'];
  report.order=options.order; report.backend=options.backend; report.window_batch=options.batch; report.compute_units=options['compute-units'];
  if(options['coreml-dir'])report.coreml_export_sha256=hash(await readFile(resolve(options['coreml-dir'],'export.json')));
  if(options.addon)report.addon_sha256=hash(await readFile(options.addon));
  for(let run=0;run<options.runs;run++){
    const child=spawnSync(resolve(options.runner),[],{input:JSON.stringify({model,order:options.order,detector:options.detector,backend:options.backend,coreml_dir:options['coreml-dir'],compute_units:options['compute-units'],fp16:options.precision==='float16',batch:options.batch,samples:options.samples,warmup:options.warmup,fixtures}),encoding:'utf8',timeout:options['timeout-ms'],maxBuffer:32*1024*1024}); // 32 MiB
    if(child.error||child.status!==0){report.errors.push({run,code:'runner_failed',exit_code:child.status,signal:child.signal,timeout:child.error?.code==='ETIMEDOUT'});break;}
    try{const result=validateResults(JSON.parse(child.stdout),fixtures,options.samples,options['target-ms']);if(result.allocation_instrumented)report.qualification_reasons_pending=['allocation_instrumented'];if(result.detector!==options.detector||result.backend!==options.backend||result.window_batch!==options.batch||result.fp16!==(options.precision==='float16')||result.compute_units!==options['compute-units'])throw Error("backend_mismatch");report.runs.push(result);}catch{report.errors.push({run,code:'invalid_runner_report'});break;}
    if(options.addon){
      const child=spawnSync(process.execPath,[fileURLToPath(import.meta.url),'--ffi-worker',resolve(options.addon)],{input:JSON.stringify({model,fixtures,samples:options.samples,warmup:options.warmup,backend:options.backend,precision:options.precision,batch:options.batch,coremlDir:options['coreml-dir'],computeUnits:options['compute-units'],detector:options.detector,targetMs:options['target-ms'],order:options.order}),encoding:'utf8',timeout:options['timeout-ms'],maxBuffer:32*1024*1024}); // 32 MiB
      if(child.error||child.status!==0){report.errors.push({run,code:'ffi_failed',exit_code:child.status,signal:child.signal,timeout:child.error?.code==='ETIMEDOUT'});break;}
      try{report.ffi_runs.push(validateResults(JSON.parse(child.stdout),fixtures,options.samples,options['target-ms']));}catch{report.errors.push({run,code:'invalid_ffi_report'});break;}
    }
  }
  const instrumented=report.runs.some(r=>r.allocation_instrumented);
  const complete=!instrumented&&!requested&&options.runs===3&&options.samples>=10000&&options.warmup>=20&&report.runs.length===3&&report.ffi_runs.length===3&&!report.errors.length;
  report.qualification_reasons=instrumented?['allocation_instrumented']:[];delete report.qualification_reasons_pending;
  for(const [condition,reason] of [[!!requested,'filtered_fixtures'],[options.runs!==3,'requires_three_runs'],[options.samples<10000,'insufficient_samples'],[options.warmup<20,'insufficient_warmup'],[report.runs.length!==3,'incomplete_native_runs'],[report.ffi_runs.length!==3,'incomplete_ffi_runs'],[!!report.errors.length,'runner_failure']])if(condition)report.qualification_reasons.push(reason);
  if([...report.runs,...report.ffi_runs].some(r=>!passesLatency(r.results,options['target-ms'])))report.qualification_reasons.push('latency_or_outcome_gate_failed');
  report.target_status=complete&&report.runs.every(r=>passesLatency(r.results,options['target-ms']))&&report.ffi_runs.every(r=>passesLatency(r.results,options['target-ms']))?'latency_qualified':'unmet';
  report.machine.load_average_end=os.loadavg();
  report.quality_status='requires_separate_validation';
  if(options.detector!=='bert')report.threshold=JSON.parse(await readFile(resolve(model,'decision.json'),'utf8')).threshold;
  await writeFile(options.output,JSON.stringify(report,null,2)+'\n',{flag:'wx',mode:0o600});
  console.log(JSON.stringify({output:options.output,target_status:report.target_status,errors:report.errors,runs:report.runs.map(r=>({backend:r.backend,accelerate:r.accelerate,batch:r.window_batch,peak_rss_bytes:r.peak_rss_bytes,results:r.results.map(x=>({id:x.id,...x.summary,stages:x.stages,replacement_only_ms:x.replacement_only_ms,errors:x.errors}))}))},null,2));
  if(report.errors.length)process.exitCode=1;
}
if(process.argv[1]&&import.meta.url===pathToFileURL(resolve(process.argv[1])).href) {
  const run=async()=>{
    if(process.argv[2]==='--ffi-worker'){
      let input='';for await(const chunk of process.stdin)input+=chunk;
      console.log(JSON.stringify(await ffiRun(process.argv[3],JSON.parse(input))));
    }else await main(process.argv.slice(2));
  };
  run().catch(()=>{console.error('benchmark_failed');process.exitCode=1;});
}
