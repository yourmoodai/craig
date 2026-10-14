import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp, readFile, rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import gate from '../tools/adversarial_reviewer/test_gate.cjs';

const runs=['verify.yml','commit-messages.yml','release.yml'].map((file,i)=>({id:i+1,head_sha:'head',event:'pull_request',path:'.github/workflows/'+file,status:'completed',conclusion:'success',run_attempt:2,html_url:'https://example.test/run/'+i}));
async function exercise({checks=runs,head='head',wait=false}={}) {
  const directory=await mkdtemp(join(tmpdir(),'craig-gate-'));
  const output=join(directory,'evidence.json');
  let polls=0;
  const github={rest:{pulls:{get:async()=>({data:{state:'open',draft:false,head:{sha:head}}})},actions:{listWorkflowRunsForRepo:'runs',listJobsForWorkflowRunAttempt:'jobs'}},
    paginate:async(method,args)=>{
      if(method==='runs')return wait && polls++===0 ? [] : checks;
      assert.equal(args.attempt_number,2);
      return [{name:'tests',conclusion:'success',steps:[{name:'unit tests',conclusion:'success'},{name:'optional evaluation',conclusion:'skipped'}]}];
    }};
  try {
    await gate({github,context:{repo:{owner:'owner',repo:'repo'}},head:'head',pr:1,output,sleep:async()=>{if(!wait)throw Error('waiting');}});
    return JSON.parse(await readFile(output,'utf8'));
  } finally {await rm(directory,{recursive:true,force:true});}
}
test('review waits for all workflows and records their attempts and skipped steps',async()=>{
  const evidence=await exercise({wait:true});
  assert.equal(evidence.head,'head');
  assert.equal(evidence.runs.length,3);
  assert.equal(evidence.runs[0].jobs[0].steps[1].conclusion,'skipped');
});
test('failed, missing, running and stale checks cannot authorize review',async()=>{
  await assert.rejects(exercise({checks:[]}),/waiting/);
  await assert.rejects(exercise({checks:runs.map(r=>({...r,head_sha:'old'}))}),/waiting/);
  await assert.rejects(exercise({checks:runs.map(r=>({...r,status:'in_progress'}))}),/waiting/);
  await assert.rejects(exercise({checks:runs.map(r=>({...r,conclusion:'failure'}))}),/did not pass/);
  await assert.rejects(exercise({head:'new'}),/stale/);
});
test('a newer unsuccessful run supersedes an older successful run',async()=>{
  await assert.rejects(exercise({checks:[...runs,{...runs[0],id:10,conclusion:'cancelled'}]}),/did not pass/);
});
