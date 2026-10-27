import {createRequire} from 'node:module';
import {mkdtemp,mkdir,realpath,rm,stat,writeFile,readFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import {tmpdir,cpus,loadavg} from 'node:os';
import {join,resolve,dirname} from 'node:path';
const require=createRequire(import.meta.url);
const temporary=await realpath(await mkdtemp(join(tmpdir(),'savior-trace-profile-')));
await mkdir(join(temporary,'profile'));
process.env.XDG_STATE_HOME=temporary;process.env.LOCALAPPDATA=temporary;process.env.CLAUDE_CONFIG_DIR=join(temporary,'profile');
const binary=resolve('target/release/savior-review'+(process.platform==='win32'?'.exe':''));
const command=(...args)=>{const r=spawnSync(binary,['traces',...args],{encoding:'utf8'});if(r.status!==0)throw Error(r.stderr);return JSON.parse(r.stdout);};
try{
 const model=await require('../native/index.cjs').loadDetector({modelDir:resolve(process.env.SAVIOR_MODEL_DIR??'.cache/models/mudes-en-base')});
 const text='You idiot, keep the test and its instructions.';const measurements=[];
 for(const mode of ['disabled','metadata','full_text']){
  if(mode==='full_text')command('enable');
  await model.rewrite(text);const samples=[];
  for(let i=0;i<5;i++){const start=performance.now();const result=mode==='disabled'?await model.rewrite(text):JSON.parse(await model.rewriteTraced(text));if(result.trace_warning)throw Error(result.trace_warning);samples.push(performance.now()-start);}
  measurements.push({mode,samples_ms:samples,rss_bytes:process.memoryUsage().rss});
 }
 const destination=join(temporary,'export.parquet');const start=performance.now();command('export',destination);const cold=performance.now()-start;
 const warm=performance.now();command('export',join(temporary,'export-warm.parquet'));
 const report={schema:1,timestamp:new Date().toISOString(),platform:process.platform,arch:process.arch,cpus:cpus().length,loadavg:loadavg(),measurements,cold_export_ms:cold,warm_export_ms:performance.now()-warm,parquet_bytes:(await stat(destination)).size,max_rss_bytes:process.resourceUsage().maxRSS*1024,scope:'synthetic short input; five samples; not detector latency qualification',identity:{}};
 for(const file of ['crates/core/src/model.rs','crates/node/src/lib.rs','crates/review/src/traces.rs','crates/review/src/inspector.rs','scripts/profile-inspector.mjs'])report.identity[file]=createHash('sha256').update(await readFile(file)).digest('hex');
 const json=JSON.stringify(report,null,2)+'\n';
 if(process.argv[2]){const output=resolve(process.argv[2]);await mkdir(dirname(output),{recursive:true});await writeFile(output,json,{flag:'wx',mode:0o600});}
 process.stdout.write(json);
}finally{await rm(temporary,{recursive:true,force:true});}
