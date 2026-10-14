import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {createRequire} from 'node:module';
import {mkdir,readFile,writeFile} from 'node:fs/promises';
import {resolve} from 'node:path';
import {pathToFileURL} from 'node:url';

export async function main(argv){
  const options={addon:'native/index.cjs',runner:'target/profiling/decision'};
  for(let i=0;i<argv.length;i+=2){const key=argv[i]?.replace(/^--/,'');if(!['addon','runner','model','kind','output'].includes(key)||argv[i+1]===undefined)throw Error('invalid_arguments');options[key]=argv[i+1];}
  if(!['lexical','crf'].includes(options.kind)||!options.model||!options.output)throw Error('invalid_arguments');
  await mkdir(options.output);
  const binding=createRequire(import.meta.url)(resolve(options.addon));const detector=options.kind==='lexical'?await binding.loadDetector({detector:'lexical',modelDir:resolve(options.model)}):await binding.loadExperimentalDetector({kind:options.kind,artifactDir:resolve(options.model)});
  const results=[];
  for(const [id,corpus] of [['corpus','tests/fixtures/corpus.jsonl'],['regression','tests/fixtures/decision-heldout.jsonl'],['evaluation_v2','tests/fixtures/decision-evaluation-v2.jsonl']]){
    const native=resolve(options.output,`${id}-native.json`);
    const child=spawnSync(options.runner,['evaluate','--corpus',corpus,'--kind',options.kind,'--model',options.model,'--output',native],{encoding:'utf8',timeout:120000});
    if(child.status!==0)throw Error('native_evaluation_failed');
    const report=JSON.parse(await readFile(native,'utf8'));const fixtures=(await readFile(corpus,'utf8')).trim().split('\n').map(JSON.parse).filter(r=>r.split==='test');
    for(const fixture of fixtures){let value;try{const r=await detector.rewrite(fixture.text);value=['ok',r.text,r.spans.map(s=>[s.start,s.end])];}catch(e){value=['error',e.message];}
      const digest=createHash('sha256').update(JSON.stringify(value)).digest('hex');const expected=report.results.find(r=>r.id===fixture.id)?.output_sha256;
      if(!expected)throw Error('incomplete_native_report');results.push({id:fixture.id,match:digest===expected});
    }
  }
  const report={version:1,kind:options.kind,boundary:'native_ffi_rewrite_text_and_utf16_offsets',model_sha256:createHash('sha256').update(await readFile(resolve(options.model,'decision.json'))).digest('hex'),examples:results.length,mismatches:results.filter(r=>!r.match).length,results};
  await writeFile(resolve(options.output,'parity.json'),JSON.stringify(report,null,2)+'\n',{flag:'wx',mode:0o600});console.log(JSON.stringify({examples:report.examples,mismatches:report.mismatches}));if(report.mismatches)process.exitCode=1;
}
if(process.argv[1]&&import.meta.url===pathToFileURL(resolve(process.argv[1])).href)main(process.argv.slice(2)).catch(()=>{console.error('decision_parity_failed');process.exitCode=1;});
