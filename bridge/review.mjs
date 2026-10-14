import {workerBinary} from './runtime.mjs';
import {connect} from 'node:net';
import {createHash} from 'node:crypto';
import {spawn} from 'node:child_process';
import {realpath, access, writeFile, readFile, lstat} from 'node:fs/promises';
import {homedir} from 'node:os';
import {join, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';

const root=fileURLToPath(new URL('..',import.meta.url));
const profile=await realpath(process.env.CLAUDE_CONFIG_DIR || join(homedir(),'.claude')).catch(()=>null);
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
async function exchange(request) {
  if (!profile) throw Error('profile_unavailable');
  let unix_endpoint;
  if(process.platform!=='win32') {
    const uid=process.geteuid();
    const directory='/tmp/craig-'+uid+'-'+createHash('sha256').update(profile).digest('hex');
    const metadata=await lstat(directory);
    if(!metadata.isDirectory() || metadata.uid!==uid || (metadata.mode&0o777)!==0o700) throw Error('review_unavailable');
    unix_endpoint=join(directory,'s');
  }
  return new Promise((done,reject)=>{
    const endpoint=process.platform==='win32' ? '\\\\.\\pipe\\craig-'+createHash('sha256').update(profile.replace(/^\\\\\?\\/, '').toLowerCase()).digest('hex') : unix_endpoint;
    const socket=connect(endpoint);
    let size=0; const chunks=[];
    socket.setTimeout(30000,()=>socket.destroy(Error('review_timeout')));
    socket.on('error',()=>reject(Error('review_unavailable')));
    socket.on('connect',()=>socket.write(JSON.stringify(request)+'\n'));
    socket.on('data',chunk=>{size+=chunk.length;if(size>16*1024*1024){socket.destroy(Error('review_response_too_large'));return;}chunks.push(chunk);}); // 16 MiB
    socket.on('end',()=>{try{const response=JSON.parse(Buffer.concat(chunks).toString('utf8'));if(response.version!==1 || response.status!=='ok') throw Error(response.code||'invalid_review_response');done(response.value);}catch{reject(Error('review_request_failed'));}});
  });
}
const artifact_path=process.env.CRAIG_MODEL_DIR ? join(process.env.CRAIG_MODEL_DIR,process.env.CRAIG_DETECTOR==='lexical'?'decision.json':'manifest.json') : null;
const artifact_hash=artifact_path ? await readFile(artifact_path).then(b=>createHash('sha256').update(b).digest('hex')).catch(()=>'') : '';
const runtime_fingerprint=createHash('sha256').update(JSON.stringify(['CRAIG_MODEL_DIR','CRAIG_DETECTOR','CRAIG_POLICY','CRAIG_THRESHOLD','CRAIG_REPLACEMENT','CRAIG_JUDGE_CONFIG'].map(key=>process.env[key]||'').concat(artifact_hash))).digest('hex');
async function start(allow_mismatch=false) {
  let current;
  try {current=await exchange({command:'status'});} catch {}
  if(current) {
    if(!allow_mismatch && current.runtime_fingerprint!==runtime_fingerprint) throw Error('review_configuration_mismatch');
    return;
  }
  const binary=workerBinary;
  await access(binary).catch(()=>{throw Error('review_not_built');});
  const child=spawn(binary,['serve'],{cwd:root,env:{...process.env,CLAUDE_CONFIG_DIR:profile},detached:true,stdio:'ignore'});
  child.on('error',()=>{});child.unref();
  for(let attempt=0;attempt<100;attempt++){await sleep(100);try{await exchange({command:'status'});return;}catch{}}
  throw Error('review_unavailable');
}
export async function request_review(request) {await start(request.command==='shutdown');return exchange(request);}

async function scan(days) {
  const state=await exchange({command:'status'});
  if(!state.consent.history) throw Error('history_not_enabled');
  const index=await exchange({command:'history_index',days});
  const {listSessions,getSessionMessages}=await import('@anthropic-ai/claude-agent-sdk');
  const entries=new Map(index.entries.map(e=>[e.session_id+':'+e.uuid,e]));
  const counts={accepted:0,duplicate:0,missing_from_sdk:entries.size,empty:0,cancelled:false,coverage:index.coverage};
  let sessions=0;
  for(let offset=0;;offset+=100) {
    const page=await listSessions({limit:100,offset,includeProgrammatic:true});
    if(!page.length) break;
    for(const session of page) {
      if(!index.entries.some(e=>e.session_id===session.sessionId)) continue;
      sessions++;
      let context=[];
      for(let skip=0;;skip+=50) {
        const messages=await getSessionMessages(session.sessionId,{limit:50,offset:skip});
        if(!messages.length) break;
        for(const message of messages) {
          const entry=entries.get(session.sessionId+':'+message.uuid);
          if(!entry || message.type!=='user') continue;
          counts.missing_from_sdk--;
          const content=message.message?.content;
          const text=typeof content==='string'?content:Array.isArray(content)?content.filter(b=>b.type==='text'&&typeof b.text==='string').map(b=>b.text).join('\n'):'';
          if(!text.trim()){counts.empty++;continue;}
          const observation={id:entry.uuid,session:entry.session_id,generation:state.consent.generation,timestamp_ms:entry.timestamp_ms,source:'history',original:text,replacement:null,assessment:null,context,authorship_verified:false};
          while(true) {
            const current=await exchange({command:'status'});
            if(!current.consent.history || current.consent.generation!==state.consent.generation){counts.cancelled=true;return {...counts,sessions};}
            const result=await exchange({command:'observe',observation});
            if(result.status==='busy'){await sleep(200);continue;}
            if(result.status==='duplicate') counts.duplicate++;else counts.accepted++;
            break;
          }
          context=[...context,text].slice(-4);
          while(Buffer.byteLength(context.join(''))>16384) context.shift(); // 16 KiB
        }
      }
    }
  }
  return {...counts,sessions,scope:'stored_text_hypothetical_replacement',originals_recoverable:false};
}
async function main() {
  let bytes=0;const chunks=[];
  for await (const chunk of process.stdin){bytes+=chunk.length;if(bytes>1048576)throw Error('invalid_request');chunks.push(chunk);} // 1 MiB
  let request;try{request=JSON.parse(Buffer.concat(chunks).toString('utf8'));}catch{throw Error('invalid_request');}
  await start(request.command==='shutdown');
  let result;
  if(request.command==='scan')result=await scan(request.days);
  else if(request.command==='export_file') {
    const data=await exchange({command:'export'});
    const path=join(profile,'craig',`metrics-${Date.now()}.json`);
    await writeFile(path,JSON.stringify(data,null,2)+'\n',{mode:0o600,flag:'wx'});
    result={path};
  } else result=await exchange(request);
  process.stdout.write(JSON.stringify({version:1,status:'ok',value:result})+'\n');
}
const entrypoint=process.argv[1] ? await realpath(process.argv[1]).catch(()=>null) : null;
if(entrypoint===fileURLToPath(import.meta.url)) main().catch(()=>{process.stdout.write('{"version":1,"status":"error","code":"review_unavailable"}\n');process.exitCode=1;});
