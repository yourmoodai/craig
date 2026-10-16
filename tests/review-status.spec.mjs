import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp, readFile, rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import snapshot from '../tools/adversarial_reviewer/test_status.cjs';

const runs=['verify.yml','commit-messages.yml','release.yml'].map((file,i)=>({id:i+1,head_sha:'head',event:'pull_request',path:'.github/workflows/'+file,status:'completed',conclusion:'success',run_attempt:2,html_url:'https://example.test/run/'+i}));
async function exercise({checks=runs,head='head'}={}) {
  const directory=await mkdtemp(join(tmpdir(),'craig-gate-'));
  const output=join(directory,'evidence.json');
  const github={rest:{pulls:{get:async()=>({data:{state:'open',draft:false,head:{sha:head}}})},actions:{listWorkflowRunsForRepo:'runs',listJobsForWorkflowRunAttempt:'jobs'}},
    paginate:async(method,args)=>{
      if(method==='runs')return checks;
      assert.equal(args.attempt_number,2);
      return [{name:'tests',conclusion:'success',steps:[{name:'unit tests',conclusion:'success'},{name:'optional evaluation',conclusion:'skipped'}]}];
    }};
  try {
    await snapshot({github,context:{repo:{owner:'owner',repo:'repo'}},head:'head',pr:1,output});
    return JSON.parse(await readFile(output,'utf8'));
  } finally {await rm(directory,{recursive:true,force:true});}
}
test('snapshot records attempts and skipped steps without waiting',async()=>{
  const evidence=await exercise();
  assert.equal(evidence.runs.length,3);
  assert.equal(evidence.runs[0].jobs[0].steps[1].conclusion,'skipped');
});
test('missing, running and failed checks do not block review',async()=>{
  assert.equal((await exercise({checks:[]})).runs[0].status,'not_started');
  assert.equal((await exercise({checks:runs.map(r=>({...r,head_sha:'old'}))})).runs[0].status,'not_started');
  assert.equal((await exercise({checks:runs.map(r=>({...r,status:'in_progress',conclusion:null}))})).runs[0].status,'in_progress');
  assert.equal((await exercise({checks:runs.map(r=>({...r,conclusion:'failure'}))})).runs[0].conclusion,'failure');
  await assert.rejects(exercise({head:'new'}),/stale/);
});
test('snapshot uses the latest attempt regardless of outcome',async()=>{
  const result=await exercise({checks:[...runs,{...runs[0],id:10,conclusion:'cancelled'}]});
  assert.equal(result.runs[0].id,10);
  assert.equal(result.runs[0].conclusion,'cancelled');
});
