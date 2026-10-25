import {resolve} from 'node:path';
import {mkdir, access} from 'node:fs/promises';
import {options, failureCode} from './common.mjs';
import {quality} from './quality.mjs';
import {e2e} from './e2e.mjs';
import {performanceScreen, profileScreen, ffiWorker} from './performance.mjs';
import {report, updateReadme} from './report.mjs';
import {save} from './corpus.mjs';

async function main() {
  const [command,...argv] = process.argv.slice(2);
  if (command==='ffi-worker') {
    let input=''; for await(const chunk of process.stdin) input+=chunk;
    console.log(JSON.stringify(await ffiWorker(JSON.parse(input)))); return;
  }
  if(argv.includes('--help') || command==='--help') {
    console.log('usage: npm run bench:<quality|e2e|performance|profile|all|report|readme> -- --output NEW_PATH [--model DIR] [--corpus FILE] [--limit 200|0] [--profile release|debug] [--host yes|no] [--samples 10] [--warmup 2] [--runs 3] [--timeout-ms 120000] [--annotations FILE] [--retry PERFORMANCE_FILE]\nquality/e2e/performance/profile: output is a new JSON file; all/report/readme: output is a directory. readme explicitly updates the README results block and evidence snapshot. profile runs stage timing separately from latency. retry replays failed performance combinations and cannot pass the full-input screen. limit is per dataset; 0 evaluates all English cases. No evaluation command downloads artifacts. Performance is a screen; use the existing qualification harness for qualification.'); return;
  }
  const config = options(argv);
  if(command==='readme') { await updateReadme(config.output); return; }
  if(command==='report') { await report(config.output); return; }
  if(command==='all') {
    await mkdir(config.output,{recursive:false});
    for(const [name,run] of [['quality',quality],['e2e',e2e],['performance',performanceScreen]]) {
      try {
        const result=await run({...config,output:resolve(config.output,`${name}.json`)});
        if(result.status!=='completed' || name==='performance' && !result.under_500_ms) process.exitCode=1;
      } catch (error) {
        await save(resolve(config.output,`${name}-failure.json`),{kind:name,status:'failed',error:failureCode(error)});
        process.exitCode=1;
      }
    }
    try { await report(config.output); } catch { process.stderr.write('report_unavailable: incomplete stages\n'); process.exitCode=1; }
    return;
  }
  const run = {quality,e2e,performance:performanceScreen,profile:profileScreen}[command];
  if(!run) throw Error('invalid_command');
  try { await access(config.output); throw Error('output_exists'); } catch(error) { if(error.code!=='ENOENT') throw error; }
  const result=await run(config);
  console.log(JSON.stringify({output:config.output,status:result.status}));
  if(result.status!=='completed' || command==='performance' && !result.under_500_ms) process.exitCode=1;
}
main().catch(error=>{console.error(failureCode(error));process.exitCode=1;});
