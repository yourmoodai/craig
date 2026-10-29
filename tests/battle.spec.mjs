import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp, writeFile, readFile, mkdir} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {csv, normalize, normalizeLicense, proposal, validate, select, fetchCorpus, digest, applyAnnotations} from '../scripts/bench/corpus.mjs';
import {classification, preservation, spanCounts, replacementSummary} from '../scripts/bench/quality.mjs';
import {validateReports, report} from '../scripts/bench/report.mjs';
import {failureCode} from '../scripts/bench/common.mjs';
import {decodeMeasurement} from '../scripts/bench/performance.mjs';

const row = (id='test-1') => ({id,dataset:'scunthorpe',upstream_id:id,family:'one',split:'test',category:'clean',language:'en',track:'profanity',
  text:'🙂 keep the tests',license:'CC0-1.0',source:'synthetic test',upstream_label:false,annotation:null});
test('CSV preserves quotes, commas, line breaks and trailing spaces',()=>{
  assert.deepEqual(csv('id,text\r\n1,"hello, ""world""\nnext "\r\n'),[{id:'1',text:'hello, "world"\nnext '}]);
  for(const bad of ['a,b\n1','a,a\n1,2','a\n"unterminated','a\n"x"junk']) assert.throws(()=>csv(bad));
});
test('importer excludes unknown licenses and preserves missing annotations',()=>{
  const source={id:'scunthorpe'};
  const input={id:'one',text:'please keep this',kind:'clean',lang:'en',source:'synthetic',license:'CC BY 4.0',expected:{flagged:false}};
  const result=normalize(source,[input,{...input,id:'two',license:'unknown'}].map(JSON.stringify).join('\n'));
  assert.equal(result.accepted[0].annotation,null);
  assert.equal(result.accepted[0].text,input.text);
  assert.deepEqual(result.excluded,[{id:'scunthorpe-two',reason:'unsupported_license'}]);
  assert.equal(normalizeLicense(undefined),null);
  assert.equal(normalizeLicense('MIT'),null);
  assert.equal(normalizeLicense('constructor'),null);
  assert.equal(normalizeLicense('__proto__'),null);
});
test('focus words are proposals, not reviewed labels; ambiguity remains unresolved',()=>{
  assert.equal(proposal('x x','x'),null);
  assert.equal(proposal('abc','missing'),null);
  assert.deepEqual(proposal('🙂 x','x'),{unit:'utf16',spans:[[3,4]],reviewed:false,origin:'upstream_focus_words'});
});
test('pinned licensed fixtures preserve upstream semantics and whitespace',async()=>{
  const fixture=JSON.parse(await readFile(new URL('fixtures/licensed-import.json',import.meta.url)));
  const clean=normalize({id:'scunthorpe'},JSON.stringify(fixture.scunthorpe)).accepted[0];
  const hate=normalize({id:'hatecheck',repository:'paul-rottger/hatecheck-data'},fixture.hatecheck_csv).accepted[0];
  assert.equal(clean.upstream_label,false);assert.equal(clean.annotation,null);
  assert.equal(hate.upstream_label,true);assert.equal(hate.text.endsWith(' '),true);
  assert.equal(hate.annotation.reviewed,false);assert.equal(validate([clean,hate]).examples,2);
});
test('annotation overlays bind the exact source text and require review evidence',()=>{
  const r=row();
  assert.throws(()=>applyAnnotations([r],[{id:r.id,text_sha256:'stale',annotation:null}]),/annotation_identity/);
  const overlay={id:r.id,text_sha256:digest(r.text),annotation:{unit:'utf16',spans:[],origin:'manual',reviewed:true,reviewer:'review-1',reviewed_at:'2026-10-20',policy:'offensive_spans',policy_version:'1'}};
  assert.equal(applyAnnotations([r],[overlay]).reviewed_spans,1);
  assert.throws(()=>applyAnnotations([r],[overlay,overlay]),/annotation_identity/);
});
test('corpus validates identity, licenses and Unicode annotation boundaries',()=>{
  assert.equal(validate([row()]).reviewed_spans,0);
  assert.throws(()=>validate([row(),row()]),/duplicate/);
  assert.throws(()=>validate([{...row(),license:'unknown'}]),/invalid_corpus/);
  assert.throws(()=>validate([{...row(),split:'train'}]),/invalid_corpus/);
  assert.throws(()=>validate([{...row(),annotation:{unit:'utf16',spans:[[1,3]],origin:'manual',reviewed:false}}]),/invalid_annotation_span/);
  assert.throws(()=>validate([{...row(),annotation:{unit:'utf16',spans:[],origin:'manual',reviewed:true}}]),/invalid_annotation/);
  assert.throws(()=>validate([{...row(),annotation:undefined}]),/invalid_annotation/);
});
test('fetch rejects corrupt cached data before import without networking',async()=>{
  const dir=await mkdtemp(join(tmpdir(),'savior-corpus-test-'));
  await writeFile(join(dir,'manifest.json'),JSON.stringify({sources:[{id:'.',revision:'a'.repeat(40),files:[{path:'source.jsonl',sha256:digest('correct')}]}]}));
  await writeFile(join(dir,'source.jsonl'),'corrupt');
  await assert.rejects(fetchCorpus(dir,join(dir,'manifest.json')),/checksum_mismatch/);
  assert.equal(await readFile(join(dir,'source.jsonl'),'utf8'),'corrupt');
});
test('installed host refuses wrappers before they can redirect the isolated profile',async()=>{
  const dir=await mkdtemp(join(tmpdir(),'savior-host-wrapper-test-'));
  const wrapper=join(dir,'claude');
  await writeFile(wrapper,'#!/bin/sh\nexit 99\n',{mode:0o700});
  const result=spawnSync('python3',['scripts/test-installed-mod.py'],{encoding:'utf8',env:{...process.env,SAVIOR_CLAUDE_BINARY:wrapper,SAVIOR_MODEL_DIR:'/synthetic/model'},timeout:10000});
  assert.equal(result.status,1);
  assert.equal(JSON.parse(result.stdout).error,'host_wrapper_requires_native_binary');
});
test('sampling is reproducible and covers category-label buckets',()=>{
  const rows=Array.from({length:30},(_,i)=>({...row(`test-${i}`),category:i%2?'a':'b',upstream_label:i%3===0}));
  const selected=select(rows,8);
  assert.deepEqual(selected,select([...rows].reverse(),8));
  assert.equal(new Set(selected.map(r=>`${r.category}:${r.upstream_label}`)).size,4);
  assert.equal(select(rows,0).length,30);
});
test('unresolved predictions do not become negatives or perfect empty scores',()=>{
  const m=classification([{expected:true,predicted:null},{expected:false,predicted:false}]);
  assert.equal(m.errors,1);assert.equal(m.scored,1);assert.equal(m.recall,null);assert.equal(m.f1,null);
  const protectedResult=classification([{expected:true,predicted:null,detection_status:'protected_span_detected'}]);
  assert.equal(protectedResult.abstentions,1);assert.equal(protectedResult.errors,0);assert.equal(protectedResult.accuracy,null);
});
test('preservation catches collateral edits outside selected spans',()=>{
  const result={text:'You , keep 42.',spans:[{start:4,end:9}],replacementCount:1};
  assert.equal(preservation('You idiot, keep 42.',result,{strategy:'omit'}),true);
  assert.equal(preservation('You idiot, keep 42.',{...result,text:'You , keep 43.'},{strategy:'omit'}),false);
  assert.deepEqual(spanCounts([{start:2,end:4}],[{start:3,end:5}]),{true_positive:0,false_positive:1,false_negative:1,annotated_units:2,removed_annotated_units:1,collateral_units:1});
});
test('replacement summaries retain separate abstention and annotation denominators',()=>{
  const rows=[{strategies:{omit:{status:'unchanged',preserved:true,parity:true}}},{strategies:{omit:{status:'protected_span_detected',parity:true}}}];
  const result=replacementSummary(rows);
  assert.equal(result.replacement.omit.successful,1);assert.equal(result.replacement.omit.abstentions,1);
  assert.equal(result.reviewed_spans.omit.examples,0);assert.equal(result.reviewed_spans.omit.removal_coverage,null);
});
test('report generation rejects missing and incompatible evidence',()=>{
  assert.throws(()=>validateReports([]),/missing_reports/);
  const identity={model:{weights:'one'},machine:{cpu:'test'},configuration:{}};
  const reports=[{kind:'quality',selection:{selected:1},rows:[{}]},{kind:'e2e',correctness:[{}],adapters:[{}],host:[{}]},
    {kind:'performance',expected_measurements:1,rows:[{}],stage:'screen',qualification:'not_qualified'}].map(r=>({...r,version:1,status:'completed',identity}));
  validateReports(reports);
  assert.throws(()=>validateReports(reports.map((r,i)=>i===1?{...r,identity:{...identity,model:{weights:'two'}}}:r)),/incompatible/);
  assert.throws(()=>validateReports(reports.map((r,i)=>i===0?{...r,rows:[]}:r)),/incomplete_quality/);
});
test('diagnostics distinguish prerequisites without exposing paths or source text',()=>{
  assert.equal(failureCode(Object.assign(Error('/private/synthetic-prompt'),{code:'ENOENT'})),'prerequisite_missing');
  assert.equal(failureCode(Error('synthetic-private-prompt')),'benchmark_execution_failed');
  assert.equal(failureCode(Error('annotation_identity_mismatch')),'annotation_identity_mismatch');
});
test('performance decoding retains inference failures without treating them as throughput',()=>{
  const fixture={id:'synthetic',text:'hi'};
  const evidence={replacement_strategy:'omit',load_ms:1,peak_rss_bytes:100,results:[{id:fixture.id,bytes:2,times_ms:[30000],outcomes:{processing_timeout:1},errors:{processing_timeout:1}}]};
  const result=decodeMeasurement(JSON.stringify(evidence),fixture,1,'omit');
  assert.equal(result.status,'failed');assert.equal(result.errors.processing_timeout,1);assert.equal(result.rewrites_per_second,null);
  assert.throws(()=>decodeMeasurement(JSON.stringify(evidence),fixture,2,'omit'),/incompatible_results/);
  assert.throws(()=>decodeMeasurement(JSON.stringify(evidence),fixture,1,'fixed_phrase'),/strategy_mismatch/);
});
test('report command publishes failed coverage honestly and never overwrites evidence',async()=>{
  const directory=await mkdtemp(join(tmpdir(),'savior-report-test-'));
  const identity={model:{'pytorch_model.bin':'synthetic'},machine:{cpu:'synthetic',platform:'test',arch:'test'},configuration:{}};
  const result={id:'one',track:'profanity',expected:true,predicted:false,strategies:{fixed_phrase:{status:'unchanged',preserved:true,parity:true}}};
  const q={version:1,kind:'quality',status:'completed',identity,selection:{selected:1},rows:[result],
    tracks:{profanity:{upstream_classification:classification([result])}},...replacementSummary([result])};
  const e={version:1,kind:'e2e',status:'failed',identity,correctness:[{id:'one',passed:false}],adapters:[{id:'one',passed:true}],host:[{status:'not_requested'}]};
  const p={version:1,kind:'performance',status:'failed',identity,stage:'screen',qualification:'not_qualified',expected_measurements:1,
    config:{runs:1,samples:1,warmup:0,timeout_ms:100},rows:[{id:'one',status:'failed',error:'child_timeout'}],under_500_ms:false,distributions:{native_fixed_phrase:null}};
  for(const stage of [q,e,p]) await writeFile(join(directory,`${stage.kind}.json`),JSON.stringify(stage));
  const snapshot=await report(directory);
  assert.equal(snapshot.performance.failed,1);assert.equal(snapshot.e2e.correctness.passed,0);
  assert.equal(snapshot.performance.load_average_start,null);
  const markdown=await readFile(join(directory,'report.md'),'utf8');
  assert.match(markdown,/0\/1 checks passed/);assert.match(markdown,/not requested/);assert.match(markdown,/did not pass/);
  assert.match(markdown,/Failure classifications: 1 `unresolved`/);
  assert.match(markdown,/contention was not recorded/);
  const saved=await readFile(join(directory,'snapshot.json'),'utf8');
  await assert.rejects(report(directory),{code:'EEXIST'});
  assert.equal(await readFile(join(directory,'snapshot.json'),'utf8'),saved);
  await mkdir(join(directory,'benchmarks/results'),{recursive:true});
  await writeFile(join(directory,'README.md'),'cute intro\n<!-- benchmark-results:start -->\n<!-- benchmark-results:end -->\ninstallation\n');
  const argv=[fileURLToPath(new URL('../scripts/bench/run.mjs',import.meta.url)),'readme','--output',directory];
  assert.equal(spawnSync(process.execPath,argv,{cwd:directory,encoding:'utf8'}).status,0);
  const readme=await readFile(join(directory,'README.md'),'utf8');
  assert.match(readme,/^cute intro/);assert.match(readme,/installation\n$/);assert.match(readme,/0\/1 checks passed/);
  snapshot.e2e.correctness.passed=100;
  await writeFile(join(directory,'snapshot.json'),JSON.stringify(snapshot));
  const tampered=spawnSync(process.execPath,argv,{cwd:directory,encoding:'utf8'});
  assert.equal(tampered.status,1);assert.match(tampered.stderr,/snapshot_summary_mismatch/);
  assert.equal(await readFile(join(directory,'README.md'),'utf8'),readme);
});
