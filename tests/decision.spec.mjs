import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,writeFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {createRequire} from 'node:module';
import {compareReports} from '../scripts/benchmark-decisions.mjs';
import {summarize} from '../scripts/benchmark-inference.mjs';
import {qualityGate} from '../scripts/qualify-decision-quality.mjs';

function report(){const row={id:'short',times_ms:[2,3],summary:summarize([2,3],500),outcomes:{rewritten:2},errors:{}};return {name:'lexical',report:{machine:{cpu:'synthetic',os:'test',arch:'test',memory_bytes:100},model_sha256:'a',runner_sha256:'b',addon_sha256:'c',threshold:0.5,precision:'float32',detector:'lexical',fixture_sha256:'d',harness_sha256:'e',target_ms:500,boundary:'resident_rewrite',order:'interleaved',node:'test',rustc:'test',lock_sha256:'f',warmup:20,errors:[],runs:[{results:[structuredClone(row)]}],ffi_runs:[{results:[structuredClone(row)]}]}};}
test('comparison rejects artifact mismatches and incomplete boundaries',()=>{
  const a=report(),b=report();b.report.model_sha256='different';assert.throws(()=>compareReports([a,b]),/artifact/);
  b.report.model_sha256='a';b.report.ffi_runs=[];assert.throws(()=>compareReports([a,b]),/incomplete/);
  const c=report();c.report.fixture_sha256='different';assert.throws(()=>compareReports([a,c]),/incompatible/);
  assert.equal(compareReports([a]).candidates[0].latency_status,'screen_pass');
  a.report.runs[0].results[0].errors={processing_timeout:1};assert.equal(compareReports([a]).candidates[0].latency_status,'unmet');
});
test('experimental FFI retains offsets, protected behavior and explicit artifact validation',async()=>{
  const dir=await mkdtemp(join(tmpdir(),'craig-decision-'));
  const binding=createRequire(import.meta.url)(resolve('native/index.cjs'));
  const artifact={version:1,kind:'lexical',policy:'offensive_spans',features:'unicode_words_context2_char2to4_v1',threshold:0.5,corpus_sha256:'a'.repeat(64),epoch:0,labels_reviewed:false,weights:{idiot:0.9},transitions:[[0,0],[0,0]]};
  try{
    await writeFile(join(dir,'decision.json'),JSON.stringify(artifact));
    const d=await binding.loadExperimentalDetector({kind:'lexical',artifactDir:dir});
    const result=await d.rewrite('🙂 You idiot, preserve café.');
    assert.equal(result.text,'🙂 You ya ya ya, preserve café.');assert.equal(result.spans[0].start,7);assert.equal(result.spans[0].end,12);assert.equal(result.offsetUnit,'utf16');
    await assert.rejects(d.rewrite('Keep `idiot` unchanged.'),/protected_span_detected/);
    const tail=await d.rewrite('x'.repeat(65530)+' idiot');assert.equal(tail.replacementCount,1);
    assert.throws(()=>d.rewrite('x'.repeat(65537)),/invalid_input/);
    await assert.rejects(binding.loadExperimentalDetector({kind:'crf',artifactDir:dir}),/model_incompatible/);
    await writeFile(join(dir,'decision.json'),JSON.stringify({...artifact,features:'unicode_words_separated_ascii_v2',weights:{idiot:0.9,moron:0.9,jackass:0.9}}));
    const runtime=await binding.loadDetector({detector:'lexical',modelDir:dir});
    assert.equal((await runtime.rewrite('You moron, retain the assertion.')).text,'You ya ya ya, retain the assertion.');
    await assert.rejects(runtime.rewrite('You i.d.i.o.t, retain the assertion.'),/protected_span_detected/);
    for(const text of ['moronicity jackass_count','i.d.i.o.t.x','préidiot','xidiot'])assert.equal((await runtime.rewrite(text)).text,text);
    const assessed=await runtime.assess('You jackass, retain café.');
    assert.equal(assessed.text,'You ya ya ya, retain café.');assert.equal(assessed.assessment.replacementCount,1);
    assert.equal((await runtime.assess('Quote "moron".')).assessment.protectedAbstention,true);
    await assert.rejects(binding.loadDetector({detector:'lexical',modelDir:dir,policy:'directed_abuse'}),/policy_unavailable/);
    await assert.rejects(binding.loadDetector({detector:'lexical',modelDir:dir,backend:'metal'}),/invalid_options/);
    const running=runtime.rewrite('x'.repeat(65530)+' idiot');
    assert.throws(()=>runtime.rewrite('You idiot.'),/busy/);await running;
    const {spawnSync}=await import('node:child_process');
    const env={...process.env,CRAIG_DETECTOR:'lexical',CRAIG_MODEL_DIR:dir,CRAIG_POLICY:'offensive_spans',CRAIG_THRESHOLD:'0.5',CRAIG_REPLACEMENT:'ya ya ya',LOG_LEVEL:'silent'};
    for(const [cmd,args] of [[process.execPath,['bridge/cli.mjs','rewrite']],['target/release/craig',['rewrite']]]){
      const r=spawnSync(cmd,args,{env,input:JSON.stringify({version:1,text:'You moron, retain café.'}),encoding:'utf8'});
      assert.equal(r.status,0,r.stderr);assert.equal(JSON.parse(r.stdout).text,'You ya ya ya, retain café.');
      const blocked=spawnSync(cmd,args,{env,input:JSON.stringify({version:1,text:'Quote "moron".'}),encoding:'utf8'});
      assert.equal(blocked.status,1);assert.equal(JSON.parse(blocked.stdout).code,'protected_span_detected');assert.equal(JSON.parse(blocked.stdout).text,undefined);
    }
    await writeFile(join(dir,'decision.json'),JSON.stringify({...artifact,threshold:0}));
    await assert.rejects(binding.loadExperimentalDetector({kind:'lexical',artifactDir:dir}),/model_incompatible/);
  }finally{await rm(dir,{recursive:true,force:true});}
});

test('quality promotion requires reviewed labels, matched artifacts and precision',()=>{
  const sample={examples:1,errors:0,labels_reviewed:true,exact_rewrite_matches:1,model_sha256:'model',corpus_sha256:'corpus',normalized_characters:{precision:1,recall:1},results:[{reviewed:true,expected_outcome:'unchanged',actual_outcome:'unchanged'}]};
  assert.equal(qualityGate([sample],sample,sample).status,'quality_pass');
  assert.ok(qualityGate([sample],{...sample,labels_reviewed:false},sample).reasons.includes('labels_require_review'));
  assert.ok(qualityGate([{...sample,exact_rewrite_matches:0}],sample,sample).reasons.includes('regression_mismatch'));
  assert.ok(qualityGate([sample],{...sample,normalized_characters:{precision:0.9,recall:1}},sample).reasons.includes('precision_below_reference'));
  assert.ok(qualityGate([sample],sample,{...sample,corpus_sha256:'other'}).reasons.includes('evaluation_corpus_mismatch'));
});

test('qualification requires complete coverage and never trusts cached summaries',()=>{
  const reports=Array.from({length:3},()=>report());
  for(const {report:r} of reports)for(const boundary of ['runs','ffi_runs']){const row=r[boundary][0].results[0];row.times_ms=Array(10000).fill(2);row.outcomes={rewritten:10000};}
  assert.equal(compareReports(reports,['short']).candidates[0].latency_status,'latency_qualified');
  assert.equal(compareReports(reports,['short','missing']).candidates[0].latency_status,'screen_pass');
  reports[0].report.runs[0].results[0].times_ms[0]=500;
  assert.equal(compareReports(reports,['short']).candidates[0].latency_status,'unmet');
  reports[0].report.runs[0].allocation_instrumented=true;
  assert.throws(()=>compareReports(reports,['short']),/instrumented/);
});

test('failed candidates remain incomplete and do not stop other experiments',async()=>{
  const {mkdir,readFile}=await import('node:fs/promises');
  const {spawnSync}=await import('node:child_process');
  const dir=await mkdtemp(join(tmpdir(),'craig-decision-failures-'));
  try{
    for(const kind of ['lexical','crf']){await mkdir(join(dir,kind));await writeFile(join(dir,kind,'decision.json'),JSON.stringify({threshold:0.5}));}
    const runner=join(dir,'runner'),addon=join(dir,'fake.node'),output=join(dir,'reports');
    await writeFile(runner,'#!/bin/sh\nexit 1\n',{mode:0o700});await writeFile(addon,'synthetic artifact');
    const r=spawnSync(process.execPath,['scripts/benchmark-decisions.mjs','--models',dir,'--output',output,'--stage','pilot','--candidates','lexical,crf','--runner',runner,'--addon',addon,'--decision-runner',runner],{encoding:'utf8',timeout:30000});
    assert.equal(r.status,0,r.stderr);const report=JSON.parse(await readFile(join(output,'comparison.json'),'utf8'));
    assert.deepEqual(report.failures.map(f=>f.name),['lexical','crf']);assert.deepEqual(report.candidates,[]);
  }finally{await rm(dir,{recursive:true,force:true});}
});
