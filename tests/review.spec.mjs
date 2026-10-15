import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp, mkdir, writeFile, readFile, rm, stat, symlink, realpath} from 'node:fs/promises';
import {spawn, spawnSync, execFileSync} from 'node:child_process';
import {randomUUID, createHash} from 'node:crypto';
import {tmpdir} from 'node:os';
import {resolve, join} from 'node:path';

const root=resolve(import.meta.dirname,'..');
const day=86400000;
test('long Unix profiles retain private isolated worker sockets', {skip:process.platform==='win32',timeout:30000}, async()=>{
  const temporary=await mkdtemp(join(tmpdir(),'craig-long-'));
  const profiles=['a','b'].map(name=>join(temporary,name.repeat(140)));
  try {
    for(const profile of profiles) await mkdir(profile);
    await invoke(profiles[0],{command:'consent',live:true,history:false});
    assert.equal((await invoke(profiles[1],{command:'status'})).consent.live,false);
    assert.equal((await invoke(profiles[0],{command:'status'})).consent.live,true);
    for(const profile of profiles) {
      const hash=createHash('sha256').update(await realpath(profile)).digest('hex');
      const directory=`/tmp/craig-${process.geteuid()}-${hash}`;
      assert.equal((await stat(directory)).mode&0o777,0o700);
      assert.equal((await stat(join(directory,'s'))).mode&0o777,0o600);
    }
  } finally {
    for(const profile of profiles) await invoke(profile,{command:'shutdown'}).catch(()=>{});
    await rm(temporary,{recursive:true,force:true});
  }
});
test('review entrypoint responds through a directory alias', async()=>{
  const temporary=await mkdtemp(join(tmpdir(),'craig-entrypoint-'));
  try {
    const alias=join(temporary,'alias');
    await symlink(root,alias,process.platform==='win32'?'junction':'dir');
    const result=spawnSync(process.execPath,[join(alias,'bridge/review.mjs')],{input:'invalid json',encoding:'utf8',timeout:10000});
    assert.equal(result.status,1);
    assert.deepEqual(JSON.parse(result.stdout),{version:1,status:'error',code:'review_unavailable'});
    const preserved=spawnSync(process.execPath,['--preserve-symlinks-main',join(alias,'bridge/review.mjs')],{input:'invalid json',encoding:'utf8',timeout:10000});
    assert.equal(preserved.status,1);
    assert.deepEqual(JSON.parse(preserved.stdout),{version:1,status:'error',code:'review_unavailable'});
    if(process.platform==='win32') {
      const script=join(alias,'bridge/review.mjs');
      const short=execFileSync('cmd.exe',['/d','/c',`for %I in ("${script}") do @echo %~sI`],{encoding:'utf8'}).trim();
      const short_result=spawnSync(process.execPath,[short],{input:'invalid json',encoding:'utf8',timeout:10000});
      assert.equal(short_result.status,1);
      assert.deepEqual(JSON.parse(short_result.stdout),{version:1,status:'error',code:'review_unavailable'});
    }
  } finally {await rm(temporary,{recursive:true,force:true});}
});
test('importing the review library does not consume stdin or print a response',()=>{
  const result=spawnSync(process.execPath,['--input-type=module','-e',"await import('./bridge/review.mjs'); console.log('imported');"],{cwd:root,input:'invalid json',encoding:'utf8',timeout:10000});
  assert.equal(result.status,0);
  assert.equal(result.stdout.trim(),'imported');
});
function invoke(profile, request, extra_env={}) {
  return new Promise((done,reject)=>{
    const child=spawn(process.execPath,['bridge/review.mjs'],{cwd:root,env:{...process.env,CLAUDE_CONFIG_DIR:profile,CRAIG_JUDGE_CONFIG:'',...extra_env},stdio:['pipe','pipe','pipe']});
    let out='',err='';
    child.stdout.on('data',c=>out+=c);child.stderr.on('data',c=>err+=c);
    child.on('error',reject);
    child.on('exit',code=>{try{assert.equal(err,'');const result=JSON.parse(out);if(code!==0)reject(Error(result.code));else done(result.value);}catch(e){reject(e);}});
    child.stdin.end(JSON.stringify(request));
  });
}
async function idle(profile) {
  for(let n=0;n<300;n++) {
    const state=await invoke(profile,{command:'status'});
    if(!state.busy)return state;
    await new Promise(r=>setTimeout(r,100));
  }
  throw Error('worker_did_not_settle');
}
async function fixture(profile, project, ages) {
  const directory=join(profile,'projects',project);await mkdir(directory,{recursive:true});
  const session=randomUUID();let parent=null;
  const records=ages.map(age=>{
    const uuid=randomUUID();
    const record={parentUuid:parent,isSidechain:false,userType:'external',cwd:`/synthetic/${project}`,sessionId:session,version:'2.1.290',gitBranch:'fixture',type:'user',uuid,timestamp:new Date(Date.now()-age*day).toISOString(),message:{role:'user',content:`synthetic private canary ${age}; You idiot, do not delete the test.`}};
    parent=uuid;return JSON.stringify(record);
  });
  await writeFile(join(directory,session+'.jsonl'),records.join('\n')+'\n');
}

test('history uses lexical selection and rejects changed artifact provenance', {timeout:120000}, async()=>{
  const profile=await mkdtemp(join(tmpdir(),'craig-review-lexical-'));
  const model=join(profile,'model');await mkdir(model);
  const artifact={version:1,kind:'lexical',policy:'offensive_spans',features:'unicode_words_separated_ascii_v2',threshold:0.5,corpus_sha256:'a'.repeat(64),epoch:0,labels_reviewed:false,weights:{idiot:0.9},transitions:[[0,0],[0,0]]};
  const env={CRAIG_DETECTOR:'lexical',CRAIG_MODEL_DIR:model,CRAIG_POLICY:'offensive_spans',CRAIG_THRESHOLD:'0.5'};
  try {
    await writeFile(join(model,'decision.json'),JSON.stringify(artifact));
    await fixture(profile,'lexical',[1]);
    await invoke(profile,{command:'consent',live:false,history:true},env);
    const scan=await invoke(profile,{command:'scan',days:3},env);assert.equal(scan.accepted,1);
    let state;
    for(let n=0;n<100;n++){
      state=await invoke(profile,{command:'status'},env);
      if(!state.busy)break;
      await new Promise(r=>setTimeout(r,50));
    }
    assert.equal(state.summary.history.assessed,1);
    assert.equal(state.summary.history.removed_characters,5);
    await writeFile(join(model,'decision.json'),JSON.stringify({...artifact,weights:{idiot:0.8}}));
    await assert.rejects(invoke(profile,{command:'status'},env),/review_unavailable/);
  } finally {
    await invoke(profile,{command:'shutdown'},env).catch(()=>{});
    await rm(profile,{recursive:true,force:true});
  }
});

test('local worker enforces consent, SDK history dates, deduplication and text-free persistence', {timeout:120000}, async()=>{
  const profile=await mkdtemp(join(tmpdir(),'craig-review-'));
  try {
    await fixture(profile,'first',[6,4,1]);
    await fixture(profile,'second',[2]);
    const initial=await invoke(profile,{command:'status'});
    assert.equal(initial.consent.live,false);assert.equal(initial.consent.history,false);
    await assert.rejects(invoke(profile,{command:'status'},{CRAIG_REPLACEMENT:'la la la'}));
    await assert.rejects(invoke(profile,{command:'scan',days:3}));
    await invoke(profile,{command:'consent',live:false,history:true});
    const first=await invoke(profile,{command:'scan',days:3});
    assert.equal(first.accepted,2);assert.equal(first.sessions,2);assert.equal(first.missing_from_sdk,0);
    let state=await idle(profile);
    assert.equal(state.summary.history.observations,2);
    const assessed=state.summary.history.assessed===2;
    const repeated=await invoke(profile,{command:'scan',days:3});
    assert.equal(repeated.duplicate,assessed?2:0);
    assert.equal(repeated.accepted,assessed?0:2);
    await idle(profile);
    const expanded=await invoke(profile,{command:'scan',days:5});
    assert.equal(expanded.accepted,assessed?1:3);assert.equal(expanded.duplicate,assessed?2:0);
    await assert.rejects(invoke(profile,{command:'scan',days:6}));
    state=await idle(profile);
    assert.equal(state.summary.history.observations,3);
    assert.equal(state.version,2);
    assert.equal(state.policy_alignment_status,'inconclusive');
    assert.ok(!('certification' in state));
    assert.ok(state.metrics.every(m=>m.authorship_verified===false));
    assert.ok(!JSON.stringify(state).includes('synthetic private canary'));
    const database=join(profile,'craig/metrics.sqlite3');
    assert.ok(!(await readFile(database)).includes(Buffer.from('synthetic private canary')));
    if(process.platform==='win32') {
      // Windows privacy is an ACL invariant, not a POSIX mode-bit invariant.
      const script = `$ErrorActionPreference='Stop';
        $sid=[System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value;
        $directory=Get-Acl -LiteralPath (Split-Path -Parent $env:CRAIG_TEST_DATABASE);
        $file=Get-Acl -LiteralPath $env:CRAIG_TEST_DATABASE;
        $entries=@($file.GetAccessRules($true,$true,[System.Security.Principal.SecurityIdentifier]) | ForEach-Object { @{sid=$_.IdentityReference.Value; type=$_.AccessControlType.ToString()} });
        @{sid=$sid; protected=$directory.AreAccessRulesProtected; entries=$entries} | ConvertTo-Json -Depth 4`;
      // Node can inherit PowerShell 7 module paths from the CI shell; Windows
      // PowerShell must construct its own compatible module search path.
      const acl_env=Object.fromEntries(Object.entries(process.env).filter(([key])=>key.toLowerCase()!=='psmodulepath'));
      acl_env.CRAIG_TEST_DATABASE=database;
      const acl=JSON.parse(execFileSync('powershell.exe',['-NoProfile','-NonInteractive','-Command',script],{encoding:'utf8',timeout:15000,env:acl_env}));
      assert.equal(acl.protected,true);
      assert.ok(acl.entries.length>0);
      assert.ok(acl.entries.every(entry=>entry.sid===acl.sid && entry.type==='Allow'));
    } else {
      assert.equal((await stat(database)).mode&0o777,0o600);
    }
    const exported=await invoke(profile,{command:'export_file'});
    assert.equal(JSON.parse(await readFile(exported.path,'utf8')).metrics.length,3);
    await invoke(profile,{command:'delete'});
    assert.deepEqual((await invoke(profile,{command:'status'})).metrics,[]);
  } finally {
    await invoke(profile,{command:'shutdown'}).catch(()=>{});
    await rm(profile,{recursive:true,force:true});
  }
});

test('native assessment returns measurements without changing replacement', {skip:!process.env.CRAIG_MODEL_DIR}, async()=>{
  const {loadDetector}=await import('../native/index.cjs');
  const detector=await loadDetector({modelDir:process.env.CRAIG_MODEL_DIR});
  const text='You idiot, do not delete the test.';
  const rewrite=await detector.rewrite(text);
  const measured=await detector.assess(text);
  assert.equal(measured.text,rewrite.text);
  assert.equal(measured.assessment.inputCharacters,text.length);
  assert.equal(measured.assessment.removedCharacters,5);
  assert.equal(measured.assessment.replacementCount,1);
  assert.ok(measured.assessment.detectorConfidence>0.5);
  const protected_result=await detector.assess('Keep `idiot` as the test fixture name.');
  assert.equal(protected_result.text,undefined);
  assert.equal(protected_result.assessment.protectedAbstention,true);
});
