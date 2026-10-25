import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { spawn, spawnSync } from 'node:child_process';
const { applyReplacements, loadDetector } = createRequire(import.meta.url)('../native/index.cjs');
test('craig ignores unrelated model environment variables', () => {
  const env = {...process.env, UNRELATED_MODEL_DIR:'/synthetic/unrelated-model', LOG_LEVEL:'silent'};
  delete env.CRAIG_MODEL_DIR;
  const result = spawnSync(process.execPath, ['bridge/cli.mjs','doctor'], {env, encoding:'utf8'});
  assert.equal(result.status, 1);
  assert.equal(JSON.parse(result.stdout).code, 'model_not_configured');
  assert.equal(result.stderr, '');
});
test('ffi replaces utf16 ranges and preserves everything else', () => {
  const result = applyReplacements('🙂 idiot, do not edit `main.rs`', [{start:3,end:8,score:0.9}]);
  assert.equal(result.text, '🙂 ya ya ya, do not edit `main.rs`');
  assert.equal(result.offsetUnit,'utf16'); assert.equal(result.replacementCount,1);
});
test('ffi rejects invalid ranges and protected source', () => {
  assert.throws(() => applyReplacements('🙂 idiot',[{start:1,end:3,score:1}]), /invalid_span/);
  assert.throws(() => applyReplacements('`idiot`',[{start:1,end:6,score:1}]), /protected_span_detected/);
  assert.throws(() => applyReplacements('idiot',[{start:0.5,end:5,score:1}]), /invalid_span/);
  assert.throws(() => applyReplacements('idiot',[{start:0,end:5,score:1.000000001}]), /invalid_span/);
  assert.throws(() => applyReplacements('\ud800 idiot',[]), /invalid_input/);
});
test('ffi supports configurable marker', () => {
  assert.equal(applyReplacements('idiot',[{start:0,end:5,score:1}],{marker:'la la'}).text, 'la la');
  assert.equal(applyReplacements('idiot',[{start:0,end:5,score:0.2}]).text, 'ya ya ya');
});
test('ffi supports omission and rejects ambiguous options', () => {
  assert.equal(applyReplacements('Keep idiot here',[{start:5,end:10,score:1}],{strategy:'omit'}).text,'Keep  here');
  assert.throws(() => applyReplacements('idiot',[{start:0,end:5,score:1}],{strategy:'omit',marker:'x'}), /invalid_options/);
  assert.throws(() => applyReplacements('idiot',[{start:0,end:5,score:1}],{strategy:'unknown'}), /invalid_options/);
});
test('ffi keeps original UTF-16 offsets across multiple Unicode gaps', () => {
  const text='🙂 idiot! café 🤦 stupid!';
  const spans=[{start:text.indexOf('stupid'),end:text.indexOf('stupid')+6,score:1},{start:text.indexOf('idiot'),end:text.indexOf('idiot')+5,score:1}];
  const result=applyReplacements(text,spans);
  assert.equal(result.text,'🙂 ya ya ya! café 🤦 ya ya ya!');
  assert.deepEqual(result.spans,spans.toReversed());
});
test('model load failure contains no path or original input', async () => {
  await assert.rejects(loadDetector({modelDir:'/private/synthetic-secret'}), {message:'model_unavailable'});
});
test('ffi rejects unknown policy before loading artifacts', async () => {
  await assert.rejects(loadDetector({modelDir:'/missing',policy:'unknown'}), {message:'invalid_options'});
});
test('ffi validates explicit execution options before loading', async () => {
  for (const execution of [{backend:'unknown'}, {precision:'unknown'}, {windowBatch:3}, {backend:'cpu',precision:'float16'}, {backend:'core_ml'}, {computeUnits:'unknown'}]) {
    await assert.rejects(loadDetector({modelDir:'/missing',...execution}), {message:'invalid_options'});
  }
});
test('bridge errors never echo original text', async () => {
  const child = spawn(process.execPath, ['bridge/cli.mjs','rewrite'], {env:{...process.env,CRAIG_MODEL_DIR:'/missing',LOG_LEVEL:'silent'}});
  let stdout='', stderr=''; child.stdout.on('data', x=>stdout+=x); child.stderr.on('data',x=>stderr+=x);
  child.stdin.end(JSON.stringify({version:1,text:'synthetic-secret-canary'}));
  const code = await new Promise(resolve=>child.on('close',resolve));
  assert.equal(code,1); assert.equal(JSON.parse(stdout).status,'error');
  assert.ok(!stdout.includes('synthetic-secret-canary')); assert.equal(stderr,'');
});
