import test from 'node:test';
import assert from 'node:assert/strict';
import {child} from '../scripts/bench/common.mjs';
import {hostStream} from '../scripts/bench/host.mjs';
import {environmentSample} from '../scripts/bench/performance.mjs';

const event = (stream, name, fields = {}) => stream.line(JSON.stringify({protocol:1,event:name,
  scenario:'missing_model',strategy:'fixed_phrase',elapsed_ms:1,...fields}));
const completion = {host_version:'2.1.290 (Claude Code)',host_sha256:'a'.repeat(64),submission_to_provider_ms:20,
  samples:1,no_request_on_failure:true,injection:'none',recovery:'retained_draft_retry'};

test('host completion requires validated recovery boundaries and discards unknown fields', () => {
  const checkpoints = [], stream = hostStream('missing_model','fixed_phrase',e=>checkpoints.push(e));
  event(stream,'failure_detected',{no_request_on_failure:true,private:'secret-canary'});
  event(stream,'draft_ready'); event(stream,'recovery_submission'); event(stream,'provider_received');
  event(stream,'completed',{...completion,private:'secret-canary'});
  const result = stream.finish({status:0,milliseconds:30});
  assert.equal(result.status,'passed');
  assert.equal(result.no_request_on_failure,true);
  assert.equal(result.evidence.recovery,'retained_draft_retry');
  assert.ok(!JSON.stringify({result,checkpoints}).includes('secret-canary'));
  assert.throws(()=>event(stream,'draft_ready'),/schema_mismatch/);
});

test('completion cannot bypass failure checks, draft observation, or event identity', () => {
  const stream = hostStream('missing_model','fixed_phrase');
  assert.throws(()=>event(stream,'completed',completion),/schema_mismatch/);
  assert.throws(()=>event(stream,'ready',{scenario:'worker_exit'}),/schema_mismatch/);
  assert.throws(()=>event(stream,'ready',{elapsed_ms:-1}),/schema_mismatch/);
  assert.throws(()=>event(stream,'failed',{reason:'secret-canary',phase:'ready'}),/schema_mismatch/);
  assert.throws(()=>stream.line('{'),/malformed_json/);
  assert.equal(stream.finish({status:0,milliseconds:1}).status,'host_failed');
});

test('host failures preserve phase and partial no-delivery evidence', () => {
  const stream = hostStream('missing_model','fixed_phrase');
  event(stream,'failure_detected',{no_request_on_failure:true});
  event(stream,'failed',{reason:'draft_not_observed',phase:'failure_detected'});
  const result = stream.finish({status:1,milliseconds:10});
  assert.equal(result.reason,'draft_not_observed');
  assert.equal(result.last_stage,'failure_detected');
  assert.equal(result.no_request_on_failure,true);
  assert.equal(result.evidence,null);
});

test('real child exit, timeout, and malformed output retain safe checkpoints', async () => {
  for (const [tail,reason] of [['process.exit(7)','process_exit'],['setInterval(()=>{},1000)','child_timeout'],['console.log("{")','malformed_json']]) {
    const checkpoints=[],stream=hostStream('missing_model','fixed_phrase',e=>checkpoints.push(e));
    const code='console.log(JSON.stringify({protocol:1,event:"ready",scenario:"missing_model",strategy:"fixed_phrase",elapsed_ms:0,private:"secret-canary"}));'+tail;
    const result=await child(process.execPath,['-e',code],{timeout:500,onLine:stream.line});
    const row=stream.finish(result);
    assert.equal(row.reason,reason);
    assert.equal(checkpoints.length,1);
    assert.ok(!JSON.stringify({row,checkpoints}).includes('secret-canary'));
  }
});

test('environment metadata is numeric machine context, not a quiet-host assertion', () => {
  const sample=environmentSample();
  assert.equal(sample.load_average.length,3);
  assert.ok(sample.load_average.every(n=>Number.isFinite(n)&&n>=0));
  assert.ok(sample.logical_cpus>0);
  assert.ok(Number.isFinite(Date.parse(sample.recorded_at)));
});
