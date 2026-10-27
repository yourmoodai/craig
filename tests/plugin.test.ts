import {Runtime} from '../hooks/runtime';
import {read_result} from '../src/protocol';
import { test, expect, mock } from 'claude-code/testing';

const original = 'You idiot, fix the test';
const replacement = 'You ya ya ya, fix the test';
const prompt = { text: original, origin: { kind: 'composer' as const }, wait: false };

test('successful responses enforce the core nonblank text contract', () => {
  for (const version of [1, 2]) {
    for (const status of ['unchanged', 'rewritten']) {
      for (const text of ['', ' ', '\t\n']) {
        expect(() => read_result({version, status, text})).toThrow('invalid_response');
      }
      expect(read_result({version, status, text: '  valid text  '}).status).toBe(status);
    }
    expect(read_result({version, status: 'error', code: 'invalid_input'}).status).toBe('error');
  }
});

test('only replacement text reaches downstream', async ($, on) => {
  let forwarded = '';
  on('process.run', ($, e) => {
    expect(e.init?.stdin).toBe(JSON.stringify({ version: 3, text: original, assess: false }));
    expect(JSON.stringify(e.argv)).not.toContain(original);
    return { value: { exitCode: 0, stdout: JSON.stringify({ version: 1, status: 'rewritten', text: replacement }), stderr: '' } };
  });
  on('ui.invalidate', () => ({ value: undefined }));
  on('prompt.submit', ($, e) => { forwarded = e.text; return { text: e.text }; });
  await $.prompt.submit(prompt);
  expect(forwarded).toBe(replacement);
});

test('process failure retains draft without forwarding', async ($, on) => {
  let forwarded = false;
  let restored = '';
  on('prompt.read', () => ({value:{text:'',cursor:0}}));
  on('prompt.fill', ($, e) => { restored=e.text; return {isFilled:true}; });
  on('process.run', () => { throw Error('synthetic private failure'); });
  on('prompt.submit', ($, e) => { forwarded = true; return { text: e.text }; });
  const result = await $.prompt.submit(prompt);
  expect(result).toHaveProperty('drop');
  expect(JSON.stringify(result)).not.toContain(original);
  expect(forwarded).toBe(false);
  expect(restored).toBe(original);
});

test('failure does not overwrite a newer draft in the prompt box', async ($, on) => {
  let restored = false;
  on('prompt.read', () => ({value:{text:'new draft',cursor:9}}));
  on('prompt.fill', () => { restored=true; return {isFilled:true}; });
  on('process.run', () => ({value:{exitCode:1,stdout:'',stderr:''}}));
  expect(await $.prompt.submit(prompt)).toHaveProperty('drop');
  expect(restored).toBe(false);
});

test('session cancellation while reading the draft prevents restoration', async ($, on) => {
  let restored = false;
  on('process.run', () => ({value:{exitCode:1,stdout:'',stderr:''}}));
  on('session.end', ($, e) => ({sessionId:e.sessionId}));
  on('prompt.read', async () => {
    await $.session.end({reason:'clear',sessionId:'synthetic-session',resume:{id:'synthetic-session'}});
    return {value:{text:'',cursor:0}};
  });
  on('prompt.fill', () => { restored=true; return {isFilled:true}; });
  expect(await $.prompt.submit(prompt)).toHaveProperty('drop');
  expect(restored).toBe(false);
});

test('an older failed submission cannot restore over a newer submission', async ($, on) => {
  let restored = false;
  let mark_started: () => void = () => {};
  let release_first: () => void = () => {};
  const started = new Promise<void>(resolve => { mark_started=resolve; });
  const release = new Promise<void>(resolve => { release_first=resolve; });
  on('process.run', async ($, e) => {
    if (JSON.parse(e.init?.stdin??'{}').text === original) {
      mark_started(); await release;
      return {value:{exitCode:1,stdout:'',stderr:''}};
    }
    return {value:{exitCode:0,stdout:JSON.stringify({version:1,status:'unchanged',text:'new draft'}),stderr:''}};
  });
  on('prompt.fill', () => { restored=true; return {isFilled:true}; });
  on('ui.invalidate', () => ({value:undefined}));
  on('prompt.submit', ($, e) => ({text:e.text}));
  const first = $.prompt.submit(prompt);
  await started;
  try { expect((await $.prompt.submit({...prompt,text:'new draft'})).text).toBe('new draft'); }
  finally { release_first(); }
  expect(await first).toHaveProperty('drop');
  expect(restored).toBe(false);
});

test('draft inspection failure remains closed without disclosing the prompt', async ($, on) => {
  on('process.run', () => ({value:{exitCode:1,stdout:'',stderr:''}}));
  on('prompt.read', () => { throw Error(original); });
  const result=await $.prompt.submit(prompt);
  expect(result).toHaveProperty('drop');
  expect(JSON.stringify(result)).not.toContain(original);
});

test('malformed responses never forward', async ($, on) => {
  on('process.run', () => ({ value: { exitCode: 0, stdout: 'invalid', stderr: '' } }));
  expect(await $.prompt.submit(prompt)).toHaveProperty('drop');
});

test('nonzero exits never forward successful-looking output', async ($, on) => {
  let forwarded = false;
  on('process.run', () => ({ value: { exitCode: 1, stdout: JSON.stringify({version:1,status:'rewritten',text:replacement}), stderr: '' } }));
  on('prompt.submit', ($, e) => { forwarded = true; return {text:e.text}; });
  expect(await $.prompt.submit(prompt)).toHaveProperty('drop');
  expect(forwarded).toBe(false);
});

test('oversized input never reaches the bridge', async ($, on) => {
  let called = false;
  on('process.run', () => { called = true; throw Error('unexpected process'); });
  expect(await $.prompt.submit({...prompt,text:'x'.repeat(65537)})).toHaveProperty('drop');
  expect(called).toBe(false);
});

test('oversized replacement never reaches submission', async ($, on) => {
  let forwarded = false;
  on('process.run', () => ({value:{exitCode:0,stdout:JSON.stringify({version:1,status:'rewritten',text:'x'.repeat(65537)}),stderr:''}}));
  on('prompt.submit', ($, e) => { forwarded = true; return {text:e.text}; });
  expect(await $.prompt.submit(prompt)).toHaveProperty('drop');
  expect(forwarded).toBe(false);
});

test('valid marker expansion can exceed the input byte limit', async ($, on) => {
  const input = 'a!'.repeat(32768);
  const output = 'ya ya ya!'.repeat(32768);
  on('process.run', () => ({value:{exitCode:0,stdout:JSON.stringify({version:1,status:'rewritten',text:output}),stderr:''}}));
  on('ui.invalidate', () => ({value:undefined}));
  on('prompt.submit', ($, e) => ({text:e.text}));
  expect((await $.prompt.submit({...prompt,text:input})).text).toBe(output);
});

test('slash commands bypass replacement', async ($, on) => {
  on('prompt.submit', ($, e) => ({ text: e.text }));
  expect(await $.prompt.submit({ ...prompt, text: '/help' })).toHaveProperty('text', '/help');
});

test('inspection command adds no transcript text or model context', async ($, on) => {
  on('ui.open', ($, e) => {expect(e.focus).toBe(true);return {value:undefined};});
  const result = await $.command.run({command:'savior',args:'',origin:{kind:'composer'},presentation:{isFullscreen:true,columns:80}});
  expect(result.text).toBeUndefined();
  expect(result.context).toBeUndefined();
});

test('ending the session cancels an in-flight replacement', async ($, on) => {
  let forwarded = false;
  let restored = false;
  on('prompt.fill', () => { restored=true; return {isFilled:true}; });
  on('session.end', ($, e) => ({sessionId:e.sessionId}));
  on('process.run', async () => {
    await $.session.end({reason:'clear',sessionId:'synthetic-session',resume:{id:'synthetic-session'}});
    return {value:{exitCode:0,stdout:JSON.stringify({version:1,status:'rewritten',text:replacement}),stderr:''}};
  });
  on('prompt.submit', ($, e) => { forwarded = true; return {text:e.text}; });
  expect(await $.prompt.submit(prompt)).toHaveProperty('drop');
  expect(forwarded).toBe(false);
  expect(restored).toBe(false);
});

test('disabled personal review neither scans history nor writes metrics', async ($, on) => {
  mock.store(on);mock.clock(on,{now:1791800000000});
  on('session.id',()=>({value:'synthetic-session'}));
  on('command.register',()=>({value:{command:'savior'}}));
  on('session.start',($,e)=>({cwd:e.cwd}));
  let calls=0;
  on('process.run',($,e)=>{calls++;expect(e.argv.at(-1)).toBe('rewrite');expect(JSON.parse(e.init?.stdin??'{}').version).toBe(3);return {value:{exitCode:0,stdout:JSON.stringify({version:1,status:'rewritten',text:replacement}),stderr:''}};});
  on('ui.invalidate',()=>({value:undefined}));on('prompt.submit',($,e)=>({text:e.text}));
  await $.session.start({cwd:'/synthetic',surface:'terminal',isInteractive:true});
  await $.prompt.submit(prompt);expect(calls).toBe(1);
});

test('metrics failure never restores an original submission', async ($,on)=>{
  mock.store(on,{personal_review_enabled:true});mock.clock(on,{now:1791800000000});
  on('session.id',()=>({value:'synthetic-session'}));on('command.register',()=>({value:{command:'savior'}}));on('session.start',($,e)=>({cwd:e.cwd}));
  const assessment={input_characters:25,removed_characters:5,replacement_count:1,removed_proportion:0.2,detector_confidence:0.9,residual_confidence:0.1,protected_abstention:false};
  on('process.run',($,e)=>{
    const request=JSON.parse(e.init?.stdin??'{}');
    if(request.command==='status')return {value:{exitCode:0,stdout:JSON.stringify({version:1,status:'ok',value:{consent:{live:true,history:false,generation:1}}}),stderr:''}};
    if(request.command==='observe')throw Error('synthetic metrics failure');
    expect(request.version).toBe(3);expect(request.assess).toBe(true);
    return {value:{exitCode:0,stdout:JSON.stringify({version:2,status:'rewritten',text:replacement,assessment}),stderr:''}};
  });
  on('ui.invalidate',()=>({value:undefined}));on('prompt.submit',($,e)=>({text:e.text}));
  await $.session.start({cwd:'/synthetic',surface:'terminal',isInteractive:true});
  expect((await $.prompt.submit(prompt)).text).toBe(replacement);
});

test('history controls default to three days and cap selection at five',async($,on)=>{
  mock.store(on);mock.clock(on,{now:1791800000000});
  const ui=await $.ui.mount({plugin:'savior',surface:'terminal',component:'Pane',requestId:'savior',props:{title:'local replacement',isFocused:true,bodyColumns:80,placement:'inline',scroll:{offset:0,bodyRows:30},view:{}}});
  await ui.press({key:'personal-metrics'});
  expect(await ui.find({type:'Text',text:/lookback: 3 days/})).toBeDefined();
  await ui.press({key:'days-5'});
  expect(await ui.find({type:'Text',text:/lookback: 5 days/})).toBeDefined();
  expect(await ui.find({key:'days-6'})).toBeUndefined();
  await ui.unmount();
});

test('rejected consent remains disabled in the metrics pane',async($,on)=>{
  mock.store(on);mock.clock(on,{now:1791800000000});
  on('process.run',()=>({value:{exitCode:1,stdout:'',stderr:''}}));
  const ui=await $.ui.mount({plugin:'savior',surface:'terminal',component:'Pane',requestId:'savior',props:{title:'local replacement',isFocused:true,bodyColumns:80,placement:'inline',scroll:{offset:0,bodyRows:30},view:{}}});
  await ui.press({key:'personal-metrics'});
  await ui.press({key:'live'});
  expect(await ui.find({type:'Text',text:/personal metrics setting could not be changed/})).toBeDefined();
  expect(await ui.find({key:'live',text:/enable live metrics/})).toBeDefined();
  await ui.unmount();
});


test('packaged runtime never uses PATH node and stays closed on setup failure', async () => {
  const runtime = new Runtime(true);
  const calls: string[][] = [];
  await runtime.start({plugin:{root:'/synthetic/plugin'},process:{run:async (args) => {
    calls.push([...args]);
    if (args[1]==='-p') return {exitCode:0,stdout:'darwin-arm64',stderr:''};
    return {exitCode:1,stdout:'{}',stderr:''};
  }}});
  expect(runtime.ready).toBe(false);
  expect(calls[0]?.[0]).toBe('/synthetic/plugin/runtime/darwin-arm64/node');
  expect(calls.length).toBe(2);
});

test('packaged runtime becomes ready only after prepare and doctor', async () => {
  const runtime = new Runtime(true);
  await runtime.start({plugin:{root:'/synthetic/plugin'},process:{run:async (args,options) => {
    if (args[1]==='-p') return {exitCode:0,stdout:'darwin-arm64',stderr:''};
    if (args[1]?.endsWith('/bridge/bootstrap.mjs')) return {exitCode:0,stdout:JSON.stringify({status:'ready',runtime_dir:'/synthetic/runtime'}),stderr:''};
    if (args[1]==='prepare') return {exitCode:0,stdout:JSON.stringify({status:'ready',model_dir:'/synthetic/cache'}),stderr:''};
    expect(options?.env?.SAVIOR_MODEL_DIR).toBe('/synthetic/cache');
    return {exitCode:0,stdout:JSON.stringify({status:'ready'}),stderr:''};
  }}});
  expect(runtime.ready).toBe(true);
});

test('trace failure warns without blocking the filtered submission', async ($,on) => {
  let forwarded='';
  on('process.run', () => ({value:{exitCode:0,stdout:JSON.stringify({version:2,status:'rewritten',text:replacement,trace_warning:'trace_not_recorded'}),stderr:''}}));
  on('ui.invalidate',()=>({value:undefined}));
  on('prompt.submit',($,e)=>{forwarded=e.text;return {text:e.text};});
  await $.prompt.submit(prompt);
  expect(forwarded).toBe(replacement);
});
