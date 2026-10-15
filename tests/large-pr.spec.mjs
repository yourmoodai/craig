import {test} from 'node:test';
import assert from 'node:assert/strict';
import policy from '../tools/adversarial_reviewer/large_pr.cjs';
import publish from '../tools/adversarial_reviewer/publish.cjs';

function fixture({lines=5001,author='awill1988',owners='* @awill1988',missing=false,base='base',head='head',number=1,permission='admin'}={}) {
  const calls=[];
  const pull={state:'open',draft:false,head:{sha:head},base:{sha:base},user:{login:author,type:'User'},additions:lines,deletions:0};
  const github={rest:{pulls:{get:async()=>({data:pull}),listReviews:'reviews',createReview:async data=>calls.push({kind:'approval',...data})},repos:{getCollaboratorPermissionLevel:async()=>({data:{permission}}),getContent:async args=>{
    assert.equal(args.ref,base);
    if(missing)throw Object.assign(Error('missing'),{status:404});
    return {data:{encoding:'base64',content:Buffer.from(owners).toString('base64')}};
  }},issues:{listComments:'comments',createComment:async data=>calls.push({kind:'comment',...data}),updateComment:async data=>calls.push({kind:'update',...data})}},paginate:async()=>[]};
  return {github,context:{repo:{owner:'yourmoodai',repo:'craig'},runId:1},core:{notice(){},setFailed(){throw Error('failed');}},head:'head',pr:number,calls,pull};
}
const evidence={head:'head',runs:['verify.yml','commit-messages.yml','release.yml'].map((file,i)=>({id:i+1,workflow:'.github/workflows/'+file,conclusion:'success'}))};

test('only more than 5000 changed lines from a base code owner qualify',async()=>{
  assert.ok(await policy.eligible(fixture()));
  for(const options of [{lines:5000},{lines:4999},{author:'other'},{permission:'read'},{owners:'* @other\n/path @awill1988'},{owners:'* @org/team'},{head:'stale'},{missing:true}]) {
    assert.equal(await policy.eligible(fixture(options)),null);
  }
});
test('initial-import exception is restricted to repo, PR, base and author',async()=>{
  const initial={missing:true,base:'f0c8b668283eef344997e45740fb82774b949e2c'};
  assert.ok(await policy.eligible(fixture(initial)));
  for(const options of [{number:2},{author:'other'},{base:'changed'}])assert.equal(await policy.eligible(fixture({...initial,...options})),null);
  const other=fixture(initial);other.context.repo.repo='other';
  assert.equal(await policy.eligible(other),null);
});
test('policy approval requires all tests and publishes an explicit exception',async()=>{
  const args=fixture();
  await publish({...args,phase:'policy',evidence});
  assert.equal(args.calls.length,2);
  assert.match(args.calls[0].body,/Model review skipped/);
  assert.equal(args.calls[1].event,'APPROVE');
  assert.equal(args.calls[1].commit_id,'head');
  for(const bad of [undefined,{...evidence,head:'old'},{...evidence,runs:evidence.runs.slice(1)}]) {
    const next=fixture();
    await assert.rejects(publish({...next,phase:'policy',evidence:bad}),/passing tests/);
    assert.deepEqual(next.calls,[]);
  }
});
test('reruns update one comment and do not repeat a current-head approval',async()=>{
  const args=fixture();
  args.github.paginate=async method=>method==='reviews'
    ? [{user:{login:'github-actions[bot]',type:'Bot'},commit_id:'head',state:'APPROVED'}]
    : [{id:7,user:{login:'github-actions[bot]',type:'Bot'},body:'<!-- craig-adversarial-review -->\nwaiting'}];
  await publish({...args,phase:'policy',evidence});
  assert.equal(args.calls.length,1);
  assert.equal(args.calls[0].kind,'update');
});
