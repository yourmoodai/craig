import {test} from 'node:test';
import assert from 'node:assert/strict';
import monitor from '../tools/ci/cancel_siblings.cjs';
const context={repo:{owner:'owner',repo:'repo'},runId:4};
const run=(id,path,extra={})=>({id,path:'.github/workflows/'+path,head_sha:'head',head_repository:{full_name:'owner/repo'},event:'push',head_branch:'develop',status:'in_progress',...extra});
async function exercise(runs,{jobs={},refresh={},conflict=false}={}) {
  const cancelled=[];
  const github={rest:{actions:{listWorkflowRunsForRepo:'runs',listJobsForWorkflowRun:'jobs',
    getWorkflowRun:async({run_id})=>({data:refresh[run_id]||runs.find(r=>r.id===run_id)}),
    cancelWorkflowRun:async({run_id})=>{if(conflict)throw Object.assign(Error('finished'),{status:409});cancelled.push(run_id);}}},
    paginate:async(method,args)=>method==='runs'?runs:(jobs[args.run_id]||[])};
  await monitor({github,context,head:'head',core:{notice(){}},sleep:async()=>{throw Error('still waiting');}});
  return cancelled;
}
test('failure cancels only matching CI runs, coordinator last',async()=>{
  const runs=[run(1,'verify.yml',{conclusion:'failure',status:'completed'}),run(2,'release.yml'),run(4,'adversarial-review.yml'),
    run(5,'verify.yml',{head_sha:'other'}),run(6,'deploy.yml'),run(7,'release.yml',{head_branch:'v1.0'}),
    run(8,'verify.yml',{head_repository:{full_name:'fork/repo'}})];
  assert.deepEqual(await exercise(runs),[2,4]);
});
test('failed job cancels siblings before its workflow finishes',async()=>{
  assert.deepEqual(await exercise([run(1,'verify.yml'),run(2,'release.yml')],{jobs:{1:[{conclusion:'failure'}]}}),[1,2]);
});
test('completion races and conflict responses are harmless',async()=>{
  const runs=[run(1,'verify.yml',{conclusion:'failure',status:'completed'}),run(2,'release.yml')];
  assert.deepEqual(await exercise(runs,{refresh:{2:{...runs[1],status:'completed'}}}),[]);
  assert.deepEqual(await exercise(runs,{conflict:true}),[]);
});
test('cancellation does not cascade and missing workflows keep monitoring',async()=>{
  await assert.rejects(exercise([run(1,'verify.yml',{status:'completed',conclusion:'cancelled'})]),/still waiting/);
});
test('coordinator exits when all checks finish successfully',async()=>{
  const runs=['verify.yml','commit-messages.yml','release.yml','adversarial-review.yml'].map((p,i)=>run(i+1,p,{status:i===3?'in_progress':'completed',conclusion:i===3?null:'success'}));
  assert.deepEqual(await exercise(runs,{jobs:{4:[{name:'adversarial verdict',status:'completed',conclusion:'success'},{name:'cancel sibling ci on failure',status:'in_progress'}]}}),[]);
});
