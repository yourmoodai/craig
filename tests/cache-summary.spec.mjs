import {test} from 'node:test';
import assert from 'node:assert/strict';
import summarize from '../scripts/release/cache-summary.cjs';

test('cache measurements use this job and namespace and preserve unavailable timings',async()=>{
  const saved={...process.env};
  Object.assign(process.env,{CACHE_JOB:'native (macos-14)',CACHE_GROUP:'native-macos-14-ARM64',CACHE_HIT:'true',GITHUB_RUN_ATTEMPT:'2',GITHUB_REF:'refs/pull/1/merge'});
  let rows;
  const summary={addHeading(){return this;},addTable(value){rows=value;return this;},addRaw(){return this;},async write(){}};
  const github={rest:{actions:{listJobsForWorkflowRunAttempt:'jobs',getActionsCacheList:'caches'}},paginate:async(method,args)=>{
    if(method==='jobs') {
      assert.equal(args.attempt_number,2);
      return [{name:'native (macos-14)',steps:[{name:'restore rust dependencies',started_at:'2026-01-01T00:00:00Z',completed_at:'2026-01-01T00:00:03Z'}]}];
    }
    assert.equal(args.ref,'refs/pull/1/merge');
    return [{key:'v0-rust-native-macos-14-ARM64-hash',size_in_bytes:100},{key:'v0-rust-coreml-macos-15-ARM64-hash',size_in_bytes:999}];
  }};
  try {
    await summarize({github,context:{repo:{owner:'a',repo:'b'},runId:1},core:{summary}});
    const values=Object.fromEntries(rows);
    assert.equal(values['Restore seconds'],'3');
    assert.equal(values['Build seconds'],'unavailable');
    assert.equal(values['Existing cache bytes for this ref and namespace'],'100');
  } finally {
    for(const key of ['CACHE_JOB','CACHE_GROUP','CACHE_HIT','GITHUB_RUN_ATTEMPT','GITHUB_REF']) {
      if(saved[key]===undefined)delete process.env[key];else process.env[key]=saved[key];
    }
  }
});
