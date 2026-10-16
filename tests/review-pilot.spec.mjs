import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import restore from '../tools/adversarial_reviewer/pilot.cjs';

const prefix='haiku45-pilot-20261015-';
function fixture({artifacts=[],jobs=[],runs=[{id:4,event:'push',head_branch:'develop'}]}={}) {
  return {context:{repo:{owner:'yourmoodai',repo:'craig'},runId:5},github:{
    rest:{actions:{listArtifactsForRepo:'artifacts',listWorkflowRuns:'runs',listJobsForWorkflowRun:'jobs',downloadArtifact:async()=>({data:Buffer.from('ledger archive')})}},
    paginate:async method=>({artifacts,jobs,runs})[method],
  }};
}
const reserved={id:10,name:prefix+'reserved-4-1',workflow_run:{id:4},expired:false};
const settled={id:11,name:prefix+'settled-4-1',workflow_run:{id:4},expired:false};

test('pilot restores reconciliation, rejects uncertain or missing history',async()=>{
  const previous=process.cwd();
  const temp=fs.mkdtempSync(path.join(os.tmpdir(),'craig-pilot-'));
  process.chdir(temp);
  try {
    await restore(fixture());
    assert.equal(JSON.parse(fs.readFileSync('review-result/reservation.json')).maximum_usd,2);
    await restore(fixture({artifacts:[reserved,settled]}));
    assert.equal(fs.readFileSync('review-result/previous-ledgers/11.zip','utf8'),'ledger archive');
    const olderIdReservation={...reserved,id:3,name:prefix+'reserved-6-1',workflow_run:{id:6}};
    const olderIdSettled={...settled,id:2,name:prefix+'settled-6-1',workflow_run:{id:6}};
    await restore(fixture({artifacts:[reserved,settled,olderIdReservation,olderIdSettled],
      runs:[{id:4,event:'push',head_branch:'develop'},{id:6,event:'push',head_branch:'develop'}]}));
    assert.equal(fs.readFileSync('review-result/previous-ledgers/2.zip','utf8'),'ledger archive');
    await assert.rejects(restore(fixture({artifacts:[reserved]})),/unresolved/);
    await assert.rejects(restore(fixture({artifacts:[reserved,{...settled,expired:true}]})),/expired/);
    await assert.rejects(restore(fixture({jobs:[{run_attempt:1,steps:[{name:'persist pilot reservation',conclusion:'success'}]}]})),/missing/);
    fs.unlinkSync('review-result/previous-ledgers/11.zip');
    await restore(fixture({artifacts:[reserved,settled],runs:[{id:4,event:'pull_request',head_branch:'evil'}]}));
    assert.equal(fs.existsSync('review-result/previous-ledgers/11.zip'),false);
  } finally {
    process.chdir(previous);
    fs.rmSync(temp,{recursive:true,force:true});
  }
});
