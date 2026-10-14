import test from 'node:test';
import assert from 'node:assert/strict';
import {summarize,fixtureText,validateResults,passesLatency} from '../scripts/benchmark-inference.mjs';
test('tail intervals remain unbounded for insufficient samples',()=>{
  assert.equal(summarize([1,2,3]).p99_interval_95_ms[1],null);
  assert.deepEqual(summarize(Array(10000).fill(10)).p99_interval_95_ms,[10,10]);
  assert.equal(summarize(Array.from({length:100},(_,i)=>i+1)).p99_ms,99);
  assert.throws(()=>summarize([NaN]));
});
test('maximum gate counts a single slow sample and the exact deadline',()=>{
  assert.equal(summarize([1,499.999]).exceedances,0);
  assert.equal(summarize([...Array(999).fill(1),500]).exceedances,1);
  assert.equal(summarize([501,500,1]).exceedances,2);
});
test('synthetic byte buckets preserve UTF-8 at the size boundary',()=>{
  for(let bytes=1;bytes<=20;bytes++){
    const text=fixtureText({pattern:'🙂 café ',bytes});
    assert.equal(Buffer.byteLength(text),bytes);assert.ok(!text.includes('�'));
  }
  assert.throws(()=>fixtureText({pattern:'',bytes:1}));
  assert.throws(()=>fixtureText({pattern:'x',bytes:65537}));
});

test('missing outcomes cannot qualify a latency run',()=>{
  const fixtures=[{id:'short',text:'x'}];
  assert.throws(()=>validateResults({results:[]},fixtures,1));
  assert.throws(()=>validateResults({results:[{id:'short',bytes:1,times_ms:[1],errors:{},outcomes:{}}]},fixtures,1));
});

test('failed runner leaves text-free unmet evidence without overwriting it',async()=>{
  const {mkdtemp,writeFile,readFile,rm}=await import('node:fs/promises');
  const {tmpdir}=await import('node:os');
  const {join}=await import('node:path');
  const {spawnSync}=await import('node:child_process');
  const directory=await mkdtemp(join(tmpdir(),'craig-benchmark-'));
  try {
    const runner=join(directory,'runner'),output=join(directory,'report.json');
    await writeFile(join(directory,'pytorch_model.bin'),'synthetic artifact');
    await writeFile(runner,'#!/bin/sh\nprintf \'{"private":"synthetic private canary"}\\n\'\n',{mode:0o700});
    const args=['scripts/benchmark-inference.mjs','--runner',runner,'--fixture','short','--samples','1','--warmup','0','--output',output];
    const options={env:{...process.env,CRAIG_MODEL_DIR:directory},encoding:'utf8'};
    assert.notEqual(spawnSync(process.execPath,args,options).status,0);
    const contents=await readFile(output,'utf8'),report=JSON.parse(contents);
    assert.equal(report.target_status,'unmet');
    assert.equal(report.errors[0].code,'invalid_runner_report');
    assert.ok(!contents.includes('synthetic private canary'));
    assert.ok(!contents.includes('You idiot'));
    assert.notEqual(spawnSync(process.execPath,args,options).status,0);
    assert.equal(await readFile(output,'utf8'),contents);
  }finally{await rm(directory,{recursive:true,force:true});}
});

test('latency gate rejects errors, unknown outcomes and a single deadline exceedance',()=>{
  const row={outcomes:{rewritten:1},errors:{},summary:summarize([499.9])};
  assert.equal(passesLatency([row]),true);
  assert.equal(passesLatency([{...row,summary:summarize([500])}]),false);
  assert.equal(passesLatency([{...row,errors:{processing_timeout:1}}]),false);
  assert.equal(passesLatency([{...row,outcomes:{processing_timeout:1}}]),false);
  assert.equal(passesLatency([{...row,profile_error:'inference_failed'}]),false);
  assert.equal(passesLatency([]),false);
});

test('explicit 500 ms target preserves strict comparison',()=>{
  const row={outcomes:{unchanged:2},errors:{},summary:summarize([1,500],500)};
  assert.equal(row.summary.exceedances,1);assert.equal(passesLatency([row],500),false);
  assert.equal(passesLatency([row],600),true);assert.throws(()=>summarize([1],NaN));
});
test('varied synthetic fixtures are reproducible and byte bounded',()=>{
  const fixture={segments:['🙂 test {n}. ','a ! b ? {n}. '],seed:17,bytes:65536};
  assert.equal(fixtureText(fixture),fixtureText(fixture));assert.equal(Buffer.byteLength(fixtureText(fixture)),65536);
  assert.notEqual(fixtureText(fixture),fixtureText({...fixture,seed:2}));
});
