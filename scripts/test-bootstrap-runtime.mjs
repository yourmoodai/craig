import {createHash,generateKeyPairSync,sign} from 'node:crypto';
import {readFile,mkdtemp,realpath,mkdir,writeFile,rm} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import assert from 'node:assert/strict';
import {prepare_runtime} from '../bridge/bootstrap.mjs';

const temporary=await realpath(await mkdtemp(join(tmpdir(),'craig-signed-runtime-')));
const variable=process.platform==='win32'?'LOCALAPPDATA':'XDG_CACHE_HOME',previous=process.env[variable];
process.env[variable]=join(temporary,'cache');
try {
 const target=process.platform+'-'+process.arch,suffix=process.platform==='win32'?'.exe':'';
 const addon={'darwin-arm64':'darwin-arm64','linux-x64':'linux-x64-gnu','win32-x64':'win32-x64-msvc'}[target];assert.ok(addon,'unsupported_platform');
 const assets=new Map();for(const [name,path] of [['craig'+suffix,'target/release/craig'+suffix],['craig-review'+suffix,'target/release/craig-review'+suffix],['craig.node',`native/craig.${addon}.node`]])assets.set(name,await readFile(path));
 const plugin=join(temporary,'plugin');await mkdir(plugin);
 const version=JSON.parse(await readFile('package.json')).version;
 const manifest=Buffer.from(JSON.stringify({schema:1,protocol:1,version,targets:{[target]:[...assets].map(([name,bytes])=>({name,asset:name,bytes:bytes.length,sha256:createHash('sha256').update(bytes).digest('hex')}))}}));
 const {publicKey,privateKey}=generateKeyPairSync('ed25519');
 await writeFile(join(plugin,'release.json'),JSON.stringify({version}));await writeFile(join(plugin,'runtime-manifest.json'),manifest);await writeFile(join(plugin,'runtime-manifest.sig'),sign(null,manifest,privateKey));await writeFile(join(plugin,'release-public.pem'),publicKey.export({type:'spki',format:'pem'}));
 let downloads=0;const start=performance.now();const runtime=await prepare_runtime(plugin,async url=>{downloads++;return new Response(assets.get(new URL(url).pathname.split('/').at(-1)));});const cold=performance.now()-start;
 const warm_start=performance.now();await prepare_runtime(plugin,()=>{throw Error('unexpected_network');});const warm=performance.now()-warm_start;assert.equal(downloads,3);
 await mkdir(join(temporary,'profile'));await mkdir(join(temporary,'empty-path'));
 const env={...process.env,PATH:join(temporary,'empty-path'),XDG_STATE_HOME:temporary,LOCALAPPDATA:temporary,CLAUDE_CONFIG_DIR:join(temporary,'profile'),CRAIG_MODEL_DIR:resolve(process.env.CRAIG_MODEL_DIR??'.cache/models/mudes-en-base')};
 const run=(binary,args,input)=>{const result=spawnSync(binary,args,{env,input,encoding:'utf8',timeout:60000});assert.equal(result.status,0,result.stderr);return JSON.parse(result.stdout);};
 assert.equal(run(join(runtime.runtime_dir,'craig'+suffix),['doctor']).status,'ready');
 assert.equal(run(join(runtime.runtime_dir,'craig-review'+suffix),['traces','status']).full_text,false);
 assert.equal(run(join(runtime.runtime_dir,'craig'+suffix),['traces','status']).full_text,false);
 const script='const addon=require(process.argv[1]); addon.loadDetector({modelDir:process.env.CRAIG_MODEL_DIR}).then(m=>m.rewrite("You idiot, keep this test.")).then(r=>console.log(JSON.stringify(r)));';
 assert.equal(run(process.execPath,['-e',script,join(runtime.runtime_dir,'craig.node')]).text,'You ya ya ya, keep this test.');
 console.log(JSON.stringify({status:'passed',target,downloads,cold_setup_ms:cold,warm_offline_setup_ms:warm,scope:'real compiled artifacts; synthetic signed local transport; empty PATH; not a public GitHub release test'}));
}finally{if(previous===undefined)delete process.env[variable];else process.env[variable]=previous;await rm(temporary,{recursive:true,force:true});}
