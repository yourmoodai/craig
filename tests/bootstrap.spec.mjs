import test from 'node:test';
import assert from 'node:assert/strict';
import {generateKeyPairSync,sign,createHash} from 'node:crypto';
import {mkdtemp,readFile,writeFile,rm,symlink,realpath,mkdir,copyFile} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {verify_manifest,download_file,private_directory,prepare_runtime} from '../bridge/bootstrap.mjs';
const hash=b=>createHash('sha256').update(b).digest('hex');
test('bootstrap entrypoint reports setup failure through a directory alias; imports stay silent',async()=>{
 const temporary=await realpath(await mkdtemp(join(tmpdir(),'savior-bootstrap-')));
 try {
  const plugin=join(temporary,'plugin with spaces');await mkdir(join(plugin,'bridge'),{recursive:true});
  await copyFile(new URL('../bridge/bootstrap.mjs',import.meta.url),join(plugin,'bridge/bootstrap.mjs'));
  const alias=join(temporary,'alias');await symlink(plugin,alias,process.platform==='win32'?'junction':'dir');
  for(const directory of [plugin,alias]) {
   const result=spawnSync(process.execPath,[join(directory,'bridge/bootstrap.mjs')],{encoding:'utf8'});
   assert.equal(result.status,1,result.stderr);
   assert.deepEqual(JSON.parse(result.stdout),{status:'error',code:'runtime_setup_failed'});
  }
  const imported=spawnSync(process.execPath,['--input-type=module','-e','import "./bridge/bootstrap.mjs"'],{cwd:alias,encoding:'utf8'});
  assert.equal(imported.status,0,imported.stderr);assert.equal(imported.stdout,'');assert.equal(imported.stderr,'');
 }finally{await rm(temporary,{recursive:true,force:true});}
});
test('only pinned version, supported platform, and signed exact file inventory are trusted',()=>{
 const {publicKey,privateKey}=generateKeyPairSync('ed25519');const key=publicKey.export({type:'spki',format:'pem'});
 const manifest={schema:1,protocol:1,version:'0.1.0',targets:{'darwin-arm64':['savior','savior-review','savior.node'].map(name=>({name,asset:name,bytes:4,sha256:hash('test')}))}};
 const bytes=Buffer.from(JSON.stringify(manifest));const signature=sign(null,bytes,privateKey);
 assert.equal(verify_manifest(bytes,signature,key,'0.1.0','darwin-arm64').version,'0.1.0');
 assert.throws(()=>verify_manifest(bytes,signature,key,'0.2.0','darwin-arm64'),/incompatible/);
 assert.throws(()=>verify_manifest(bytes,signature,key,'0.1.0','linux-arm64'),/unsupported/);
 assert.throws(()=>verify_manifest(Buffer.from('{}'),signature,key,'0.1.0','darwin-arm64'),/signature/);
 const wrong=generateKeyPairSync('ed25519').publicKey.export({type:'spki',format:'pem'});assert.throws(()=>verify_manifest(bytes,signature,wrong,'0.1.0','darwin-arm64'),/signature/);
 manifest.targets['darwin-arm64'][0].asset='../escape';const bad=Buffer.from(JSON.stringify(manifest));assert.throws(()=>verify_manifest(bad,sign(null,bad,privateKey),key,'0.1.0','darwin-arm64'),/manifest/);
});
test('download resumes, validates, rejects tampering, and reuses cache offline',async()=>{
 const temporary=await mkdtemp(join(tmpdir(),'savior-bootstrap-'));try{
 const path=join(temporary,'savior');const entry={bytes:8,sha256:hash('testdata')};
 await writeFile(path+'.partial','test');let calls=0;
 await download_file('https://example.invalid/asset',path,entry,async(_url,options)=>{calls++;assert.equal(options.headers.Range,'bytes=4-');return new Response('data',{status:206,headers:{'content-range':'bytes 4-7/8'}});});
 assert.equal(await readFile(path,'utf8'),'testdata');assert.equal(calls,1);
 await download_file('https://example.invalid/asset',path,entry,()=>{throw Error('offline');});
 await writeFile(path,'tampered');await assert.rejects(download_file('https://example.invalid/asset',path,entry,async()=>new Response('badbytes')),/digest/);
 assert.equal(await readFile(path,'utf8'),'tampered');
 await assert.rejects(download_file('https://example.invalid/asset',path,entry,async()=>new Response('toolongdata')),/too_large/);
 }finally{await rm(temporary,{recursive:true,force:true});}
});
test('private directory and artifact refuse symlinks',async()=>{
 if(process.platform==='win32')return;
 const temporary=await realpath(await mkdtemp(join(tmpdir(),'savior-bootstrap-')));try{
 await private_directory(join(temporary,'private'));
 await symlink(join(temporary,'private'),join(temporary,'link'));
 await assert.rejects(private_directory(join(temporary,'link','nested')),/unsafe/);
 const target=join(temporary,'target');await writeFile(target,'test');await symlink(target,join(temporary,'artifact'));
 await assert.rejects(download_file('https://example.invalid',join(temporary,'artifact'),{bytes:4,sha256:hash('test')}),/unsafe/);
 }finally{await rm(temporary,{recursive:true,force:true});}
});
test('signed first activation, concurrent setup, crash recovery and offline reuse',async()=>{
 const temporary=await realpath(await mkdtemp(join(tmpdir(),'savior-bootstrap-')));
 const variable=process.platform==='win32'?'LOCALAPPDATA':'XDG_CACHE_HOME';const previous=process.env[variable];process.env[variable]=join(temporary,'cache');
 try {
  const plugin=join(temporary,'plugin');await mkdir(plugin);
  const {publicKey,privateKey}=generateKeyPairSync('ed25519');const target=process.platform+'-'+process.arch;const suffix=process.platform==='win32'?'.exe':'';
  const names=['savior'+suffix,'savior-review'+suffix,'savior.node'];
  const manifest={schema:1,protocol:1,version:'0.1.0',targets:{[target]:names.map(name=>({name,asset:name,bytes:4,sha256:hash('test')}))}};
  const bytes=Buffer.from(JSON.stringify(manifest));
  await writeFile(join(plugin,'release.json'),JSON.stringify({version:'0.1.0'}));await writeFile(join(plugin,'runtime-manifest.json'),bytes);await writeFile(join(plugin,'runtime-manifest.sig'),sign(null,bytes,privateKey));await writeFile(join(plugin,'release-public.pem'),publicKey.export({type:'spki',format:'pem'}));
  let downloads=0;const fetcher=async url=>{assert.ok(url.startsWith('https://github.com/yourmoodai/savior/releases/download/v0.1.0/'));downloads++;return new Response('test');};
  const [first,second]=await Promise.all([prepare_runtime(plugin,fetcher),prepare_runtime(plugin,fetcher)]);assert.equal(first.runtime_dir,second.runtime_dir);assert.equal(downloads,3);
  const offline=()=>{throw Error('offline');};assert.deepEqual(await prepare_runtime(plugin,offline),first);
  const exited=spawnSync(process.execPath,['-e','process.exit(0)']);await writeFile(join(first.runtime_dir,'setup.lock'),String(exited.pid));assert.deepEqual(await prepare_runtime(plugin,offline),first);
  await writeFile(join(first.runtime_dir,'savior.node'),'evil');await assert.rejects(prepare_runtime(plugin,offline),/offline/);
  await writeFile(join(plugin,'runtime-manifest.sig'),Buffer.alloc(64));await assert.rejects(prepare_runtime(plugin,fetcher),/signature/);assert.equal(downloads,3);
 }finally{if(previous===undefined)delete process.env[variable];else process.env[variable]=previous;await rm(temporary,{recursive:true,force:true});}
});
