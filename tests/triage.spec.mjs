import test from 'node:test';
import assert from 'node:assert/strict';
import {child, sameIdentity} from '../scripts/bench/common.mjs';
import {measurementStream, finishMeasurement, decodeMeasurement, matchedComparisons} from '../scripts/bench/performance.mjs';
import {applyAnnotations, digest, annotationFor} from '../scripts/bench/corpus.mjs';
import {annotationScore, reviewedPolicySummary} from '../scripts/bench/quality.mjs';
import {reserveGroups, qualityInventory, performanceFailures} from '../scripts/bench/triage.mjs';

const fixture={id:'one',text:'synthetic'};
const event=(stream,value)=>stream.line(JSON.stringify(value));
const loaded=stream=>event(stream,{event:'loaded',load_ms:1});
const sample=(stream,outcome='unchanged',index=0)=>event(stream,{event:'sample',id:'one',index,elapsed_ms:2,outcome});
const row=(id='one',text='🙂 keep it',family=id)=>({id,text,family,dataset:'scunthorpe',upstream_id:id,track:'profanity',
  category:'clean',language:'en',split:'test',source:'synthetic',license:'CC0-1.0',upstream_label:false,annotation:null});
const overlay=(r,policy='offensive_spans',spans=[])=>({id:r.id,text_sha256:digest(r.text),annotation:{unit:'utf16',spans,origin:'manual',
  policy,policy_version:'1',reviewed:true,review_status:'reviewed',reviewer:'synthetic-review',reviewed_at:'2026-10-20',classification:'aligned'}});

test('partial samples survive a real child watchdog without retaining raw output',async()=>{
  const checkpoints=[], stream=measurementStream(fixture,2,0,e=>checkpoints.push(e));
  const script='console.log(JSON.stringify({event:"loaded",load_ms:1}));console.log(JSON.stringify({event:"sample",id:"one",index:0,elapsed_ms:2,outcome:"unchanged",private:"canary"}));setInterval(()=>{},1000)';
  const result=await child(process.execPath,['-e',script],{timeout:500,onLine:stream.line});
  const report=finishMeasurement(result,stream,fixture,2,'omit');
  assert.equal(report.cause,'worker_timeout');assert.equal(report.failure_phase,'rewrite');
  assert.deepEqual(report.times_ms,[2]);assert.equal(report.censored,true);assert.equal(report.completed_samples,1);
  assert.ok(!JSON.stringify({report,checkpoints}).includes('canary'));
});
test('stream rejects malformed JSON, wrong fixture, duplicates, and premature samples',()=>{
  const stream=measurementStream(fixture,1,1);
  assert.throws(()=>stream.line('{'),/malformed_json/);
  assert.throws(()=>sample(stream),/schema/);
  loaded(stream);assert.throws(()=>sample(stream),/schema/);
  event(stream,{event:'warmup',id:'one'});sample(stream);
  assert.throws(()=>sample(stream),/schema/);
});
test('truncated and empty reports are distinct from processing timeouts and process exits',()=>{
  const stream=measurementStream(fixture,1,0);loaded(stream);sample(stream,'processing_timeout');
  const row={id:'one',bytes:9,times_ms:[2],outcomes:{processing_timeout:1},errors:{processing_timeout:1}};
  event(stream,{replacement_strategy:'omit',load_ms:1,peak_rss_bytes:10,results:[row]});
  const result=finishMeasurement({status:0,stdout:'present'},stream,fixture,1,'omit');
  assert.equal(result.cause,'processing_timeout');assert.equal(result.censored,false);assert.equal(result.rewrites_per_second,null);
  assert.equal(finishMeasurement({status:73,stdout:''},stream,fixture,1,'omit').cause,'process_exit');
  const empty=measurementStream(fixture,1,0);
  assert.equal(finishMeasurement({status:0,stdout:''},empty,fixture,1,'omit').cause,'empty_report');
  assert.equal(finishMeasurement({status:0,stdout:'{'},empty,fixture,1,'omit').cause,'truncated_report');
});
test('measurement decoding ignores separate profile failures but validates error counts',()=>{
  const data={replacement_strategy:'omit',load_ms:1,peak_rss_bytes:10,
    results:[{id:'one',bytes:9,times_ms:[2],outcomes:{unchanged:1},errors:{},profile_error:'processing_timeout',private:'canary'}]};
  const decoded=decodeMeasurement(JSON.stringify(data),fixture,1,'omit');
  assert.equal(decoded.status,'completed');assert.ok(!JSON.stringify(decoded).includes('canary'));
  data.results[0].errors={processing_timeout:1};
  assert.throws(()=>decodeMeasurement(JSON.stringify(data),fixture,1,'omit'),/schema/);
});
test('identity changes and unmatched failures cannot support comparisons',()=>{
  assert.equal(sameIdentity({worktree_sha256:'a'},{worktree_sha256:'b'}),false);
  assert.equal(sameIdentity({harness:{a:'1'}},{harness:{a:'2'}}),false);
  const base={id:'one',run:1,boundary:'native',status:'completed',summary:{p50_ms:2}};
  assert.equal(matchedComparisons([{...base,strategy:'fixed_phrase'},{...base,strategy:'omit',status:'failed'}])[0].comparable,false);
});
test('policy overlays are independent, atomic, and cannot promote unresolved labels',()=>{
  const r=row(), a=overlay(r), b=overlay(r,'directed_abuse',[[3,7]]);
  applyAnnotations([r],[a,b]);
  assert.deepEqual(annotationFor(r,'offensive_spans').spans,[]);assert.deepEqual(annotationFor(r,'directed_abuse').spans,[[3,7]]);
  assert.equal(r.annotation,null);
  assert.throws(()=>applyAnnotations([r],[{...a,annotation:{...a.annotation,spans:[[3,7]]}}]),/conflict/);
  for(const bad of [{...a.annotation,policy_version:undefined},{...a.annotation,review_status:'unresolved'},
    {...a.annotation,classification:'unresolved'},{...a.annotation,spans:[[1,3]]}]) {
    const fresh=row();assert.throws(()=>applyAnnotations([fresh],[{...a,annotation:bad}]));assert.equal(fresh.annotations,undefined);
  }
});
test('independent span oracle measures removal and collateral by policy',()=>{
  const actual={text:'🙂 ',spans:[{start:3,end:10}]};
  const score=annotationScore('🙂 keep it',actual,{spans:[[3,7]]},{strategy:'omit'});
  assert.equal(score.removed_annotated_units,4);assert.equal(score.collateral_units,3);assert.equal(score.exact,false);
  const summaries=reviewedPolicySummary([{reviewed_policies:['directed_abuse'],strategies:{omit:{status:'unchanged',annotations:{directed_abuse:score}}}}]);
  assert.equal(summaries.directed_abuse.omit.examples,1);assert.equal(summaries.offensive_spans.omit.examples,0);
});
test('reservation keeps transitive duplicate/template groups out of holdout',()=>{
  const rows=[row('a','text-a','family-a'),row('b','text-b','family-a'),row('c','text-b','family-c'),row('d','text-d','family-d')];
  const parts=reserveGroups(rows,['a']);
  assert.deepEqual(parts.map(r=>r.partition),['diagnostic','diagnostic','diagnostic','reserved_evaluation']);
});
test('triage prioritizes disagreement and abstention without inventing reviewed classifications',()=>{
  const a=row('a'),b=row('b'),c=row('c');
  const result=r=>({id:r.id,dataset:r.dataset,category:r.category,expected:false,predicted:false});
  const inventory=qualityInventory([a,b,c],{rows:[result(a),{...result(b),predicted:true},{...result(c),predicted:null,detection_status:'protected_span_detected'}]});
  assert.deepEqual(inventory.map(r=>r.priority),[0,0,1]);
  assert.ok(inventory.every(r=>r.policies.offensive_spans.classification==='unresolved'));
  assert.throws(()=>qualityInventory([a],{rows:[{...result(a),expected:true}]}),/identity/);
});
test('historical failures are hypotheses until replay provides a cause',()=>{
  const r={id:'unicode',run:1,boundary:'native',strategy:'omit',status:'failed',error:'child_timeout'};
  assert.equal(performanceFailures({rows:[r]})[0].certainty,'unresolved');
  assert.equal(performanceFailures({rows:[r]},{rows:[{...r,status:'completed'}]})[0].certainty,'consistent_with_harness_repair');
  assert.equal(performanceFailures({rows:[r]},{rows:[{...r,cause:'processing_timeout'}]})[0].classification,'runtime_deadline');
});

test('repeated replays retain intermittent failures instead of replacing them with success',()=>{
  const r={id:'token_dense',run:1,boundary:'native',strategy:'omit',status:'failed',cause:'processing_timeout'};
  const result=performanceFailures({rows:[r]},[{rows:[r]},{rows:[{...r,status:'completed',cause:null}]}])[0];
  assert.equal(result.classification,'intermittent_completion');
  assert.equal(result.attempts.length,2);
  assert.equal(result.observed_cause,'processing_timeout');
});

test('review priorities identify identity, context, evasion, and abstention without approving labels',()=>{
  const rows=['contextual','ident_neutral_nh','evasion','clean'].map((category,i)=>({...row(String(i)),category,upstream_label:category==='evasion'}));
  const inventory=qualityInventory(rows,{rows:rows.map(r=>({id:r.id,dataset:r.dataset,category:r.category,expected:r.upstream_label,
    predicted:r.category==='clean'?null:!r.upstream_label,detection_status:r.category==='clean'?'protected_span_detected':'completed'}))});
  assert.deepEqual(inventory.map(r=>r.review_focus),['identity_false_positive','context_false_positive','evasion_false_negative','abstention']);
  assert.ok(inventory.every(r=>r.policies.directed_abuse.review_status==='unresolved'));
  const cleanEvasion={...row('clean-evasion'),category:'evasion'};
  assert.equal(qualityInventory([cleanEvasion],{rows:[{id:cleanEvasion.id,dataset:cleanEvasion.dataset,category:'evasion',expected:false,predicted:true}]})[0].review_focus,'other');
  assert.throws(()=>reserveGroups(rows,['absent']),/identity/);
  assert.throws(()=>qualityInventory(rows,{rows:[{id:'0',dataset:rows[0].dataset,category:rows[0].category,expected:false}]}),/identity/);
});

test('native runner separates measurement and profiling and retains failed stage evidence', {skip:!process.env.CRAIG_MODEL_DIR}, async()=>{
  const request={model:process.env.CRAIG_MODEL_DIR,backend:'cpu',batch:1,samples:1,warmup:0,fixtures:[{id:'one',text:'Please keep the test.'}]};
  for(const phase of ['measure','profile']) {
    const result=await child('target/release/examples/inference',[],{input:JSON.stringify({...request,phase})});
    assert.equal(result.status,0);
    const row=JSON.parse(result.stdout).results[0];
    assert.equal(row.times_ms.length,phase==='measure'?1:0);
    assert.equal(row.stages!==undefined,phase==='profile');
    if(phase==='profile') assert.equal(row.stages.completed_windows,row.stages.windows);
  }
  const result=await child('target/release/examples/inference',[],{input:JSON.stringify({...request,phase:'profile',fixtures:[{id:'one',text:'x'.repeat(65537)}]})});
  const row=JSON.parse(result.stdout).results[0];
  assert.equal(row.profile_error,'invalid_input');assert.equal(row.stages.completed_windows,0);assert.equal(row.replacement_only_ms,null);
});
