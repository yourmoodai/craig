import {createRequire} from 'node:module';
import {readFile} from 'node:fs/promises';
import {appendFileSync} from 'node:fs';
import {json, save, digest} from './corpus.mjs';
import {child, environment, identity, sameIdentity, strategies, strategyName, errorCode} from './common.mjs';
import {preservation} from './quality.mjs';
import {hostStream} from './host.mjs';

export async function e2e(config) {
  const binding = createRequire(import.meta.url)('../../native/index.cjs');
  const fixtures = await json('tests/fixtures/battle.json');
  const report = {version:1,kind:'e2e',identity:await identity(config),fixture_sha256:digest(await readFile('tests/fixtures/battle.json')),
    status:'running',correctness:[],adapters:[],host:[],diagnostics:[],errors:[]};
  await save(`${config.output}.identity.json`,{version:1,kind:'e2e',identity:report.identity});
  for (const replacement of strategies) {
    const marker = replacement.strategy==='omit'?'':replacement.marker??'ya ya ya';
    for (const fixture of fixtures) {
      let passed = false, status;
      try {
        const result = binding.applyReplacements(fixture.text,fixture.spans.map(([start,end])=>({start,end,score:1})),replacement);
        passed = !fixture.error && result.text===fixture.template.replaceAll('{marker}',marker);
        status = result.replacementCount?'rewritten':'unchanged';
      } catch (error) { status=errorCode(error); passed=status===fixture.error; }
      report.correctness.push({id:fixture.id,strategy:strategyName(replacement),passed,status});
    }
    for (const bytes of [1018,1020,1022,65536]) {
      const prefix = 'a '.repeat(Math.floor((bytes-6)/2));
      const text = prefix+'idiot!'+(bytes%2?' ':'');
      const result = binding.applyReplacements(text,[{start:prefix.length,end:prefix.length+5,score:1}],replacement);
      report.correctness.push({id:`boundary_${bytes}`,strategy:strategyName(replacement),passed:result.text===prefix+marker+'!',status:'rewritten'});
    }
    const fixture = fixtures.find(f=>f.model);
    const cases = [
      {id:'rewrite',request:{version:1,text:fixture.text},expected:fixture.template.replaceAll('{marker}',marker)},
      {id:'clean',request:{version:1,text:fixtures[0].text},expected:fixtures[0].text},
      {id:'protected',request:{version:1,text:'You `idiot`, do not delete the test.'},error:'protected_span_detected'},
      {id:'invalid_request',request:{version:9,text:'synthetic-secret-canary'},error:'invalid_request'},
      {id:'invalid_configuration',request:{version:1,text:'synthetic-secret-canary'},error:'invalid_options',env:{SAVIOR_REPLACEMENT_STRATEGY:'invalid'}},
      {id:'missing_model',request:{version:1,text:'synthetic-secret-canary'},error:'model_unavailable',env:{SAVIOR_MODEL_DIR:'.cache/missing-benchmark-model'}}
    ];
    for (const [adapter,command,args] of [['node_cli',process.execPath,['bridge/cli.mjs','rewrite']],['rust_cli',`target/${config.profile}/savior${process.platform==='win32'?'.exe':''}`,['rewrite']]]) {
      for (const test of cases) {
        const result = await child(command,args,{input:JSON.stringify(test.request),env:{...environment(config.model,replacement),...test.env},timeout:config['timeout-ms']});
        let body; try { body=JSON.parse(result.stdout); } catch {}
        const passed = !result.error && (test.error
          ? result.status!==0 && body?.code===test.error && !Object.hasOwn(body,'text') && !result.stdout.includes(test.request.text)
          : result.status===0 && body?.text===test.expected);
        report.adapters.push({id:test.id,adapter,strategy:strategyName(replacement),passed,
          status:result.error??body?.code??body?.status??'invalid_response',process_ms:result.milliseconds});
      }
    }
    if (config.host==='yes') {
      for(const scenario of ['success','protected','missing_model','processing_timeout','worker_exit']) {
      process.stderr.write(`installed host ${strategyName(replacement)} ${scenario}\n`);
      const stream = hostStream(scenario,replacement.strategy,event => appendFileSync(`${config.output}.host-events.jsonl`,
        JSON.stringify({...event,strategy_variant:strategyName(replacement)})+'\n',{mode:0o600}));
      const result = await child('python3',['scripts/test-installed-mod.py'],{env:{...environment(config.model,replacement),SAVIOR_HOST_SCENARIO:scenario},
        timeout:Math.max(config['timeout-ms'],180000),onLine:stream.line});
      const row = {scenario,strategy:strategyName(replacement),...stream.finish(result)};
      report.host.push(row);
      await save(`${config.output}.host-${strategyName(replacement)}-${scenario}.json`,row);
      }
    } else report.host.push({strategy:strategyName(replacement),status:'not_requested'});
  }
  const probeBytes=await readFile('tests/fixtures/context-probes.json');
  report.probe_sha256=digest(probeBytes);
  const detector=await binding.loadDetector({modelDir:config.model,policy:'offensive_spans',threshold:0.5,backend:'cpu'});
  for(const probe of JSON.parse(probeBytes)) for(const replacement of strategies) {
    const row={id:probe.id,family:probe.family,strategy:strategyName(replacement),review_status:'unresolved'};
    try {
      const result=await detector.rewrite(probe.text,replacement);
      row.status=result.replacementCount?'rewritten':'unchanged';
      row.selected_units=result.spans.reduce((n,s)=>n+s.end-s.start,0);
      row.preserved=preservation(probe.text,result,replacement);
    } catch(error) { row.status=errorCode(error); }
    report.diagnostics.push(row);
  }
  report.identity_end=await identity(config);
  report.identity_stable=sameIdentity(report.identity,report.identity_end);
  report.status = !report.identity_stable || report.correctness.some(r=>!r.passed) || report.adapters.some(r=>!r.passed) || report.host.some(r=>!['passed','not_requested'].includes(r.status)) ||
    report.diagnostics.some(r=>r.status!=='protected_span_detected' && (!['rewritten','unchanged'].includes(r.status) || !r.preserved)) ? 'failed':'completed';
  await save(config.output,report);
  return report;
}
