import {spawn, spawnSync} from 'node:child_process';
import {readFile, readdir} from 'node:fs/promises';
import os from 'node:os';
import {resolve} from 'node:path';
import {digest} from './corpus.mjs';

export const strategies = [{strategy:'fixed_phrase'}, {strategy:'fixed_phrase',marker:'please'}, {strategy:'omit'}];
export const strategyName = options => options.marker === undefined ? options.strategy : 'custom_phrase';
export function options(argv) {
  const result = {model:process.env.CRAIG_MODEL_DIR ?? '.cache/models/mudes-en-base',
    corpus:'.cache/bench-corpus/corpus.json', output:undefined, limit:200, profile:'release', host:'yes',
    samples:10, warmup:2, runs:3, 'timeout-ms':120000, annotations:undefined,
    retry:undefined};
  for (let i = 0; i < argv.length; i += 2) {
    const key = argv[i]?.replace(/^--/,'');
    if (!Object.hasOwn(result,key) || argv[i+1] === undefined) throw Error('invalid_arguments');
    result[key] = argv[i+1];
  }
  for (const key of ['limit','samples','warmup','runs','timeout-ms']) {
    result[key] = Number(result[key]);
    if (!Number.isSafeInteger(result[key]) || result[key] < (['limit','warmup'].includes(key) ? 0 : 1)) throw Error('invalid_arguments');
  }
  if (!['release','debug'].includes(result.profile) || !['yes','no'].includes(result.host) || !result.output || result.runs > 3 || result.samples > 100000 || result.warmup > 1000 || result['timeout-ms'] > 2147483647) throw Error('invalid_arguments');
  return result;
}
export function child(command, args, {input, env = process.env, timeout = 120000, onLine} = {}) {
  return new Promise(resolveResult => {
    const started = performance.now();
    const process = spawn(command,args,{env,stdio:['pipe','pipe','pipe']});
    let stdout = '', pending = '', overflow = false, timed_out = false, protocol_error = null;
    const timer = setTimeout(() => { timed_out = true; process.kill('SIGKILL'); },timeout);
    process.stdout.setEncoding('utf8');
    process.stdout.on('data', bytes => {
      if (Buffer.byteLength(stdout) + Buffer.byteLength(bytes) > 32*1024*1024) { overflow = true; process.kill('SIGKILL'); return; } // 32 MiB
      stdout += bytes;
      if (onLine) {
        pending += bytes;
        let end;
        while ((end=pending.indexOf('\n'))>=0) {
          const line=pending.slice(0,end); pending=pending.slice(end+1);
          try { onLine(line); } catch(error) { protocol_error=['malformed_json','schema_mismatch','unexpected_event'].includes(error.message)?error.message:'invalid_stream'; process.kill('SIGKILL'); }
        }
      }
    });
    process.stderr.resume();
    process.stdin.on('error', () => {});
    process.on('error', () => { clearTimeout(timer); resolveResult({status:null,error:'child_unavailable',stdout:'',milliseconds:performance.now()-started}); });
    process.on('close', (status,signal) => { clearTimeout(timer); resolveResult({status,signal,error:timed_out?'child_timeout':overflow?'output_limit':protocol_error,stdout,milliseconds:performance.now()-started}); });
    process.stdin.end(input);
  });
}
export function environment(model, replacement) {
  const env = {...process.env,CRAIG_MODEL_DIR:resolve(model),CRAIG_DETECTOR:'bert',CRAIG_POLICY:'offensive_spans',CRAIG_THRESHOLD:'0.5',
    CRAIG_REPLACEMENT_STRATEGY:replacement.strategy,LOG_LEVEL:'silent'};
  delete env.CRAIG_REPLACEMENT;
  if (replacement.marker !== undefined) env.CRAIG_REPLACEMENT = replacement.marker;
  return env;
}
export async function identity(config) {
  const git = args => { const r = spawnSync('git',args,{encoding:'utf8'}); if(r.status !== 0) throw Error('git_identity_unavailable'); return r.stdout; };
  const status = git(['status','--porcelain']);
  const untracked = git(['ls-files','--others','--exclude-standard','-z']).split('\0').filter(Boolean).sort();
  const changes = [git(['diff','HEAD','--binary'])];
  for (const path of untracked) changes.push(`${path}:${digest(await readFile(path))}`);
  const artifacts = {};
  for (const name of (await readdir(config.model)).sort()) {
    if (/\.(json|bin|txt|safetensors)$/.test(name)) artifacts[name] = digest(await readFile(resolve(config.model,name)));
  }
  const addon = (await readdir('native')).filter(name => name.endsWith('.node') && name.includes(`${process.platform}-${process.arch}`));
  if (addon.length !== 1) throw Error('native_addon_unavailable');
  const code = {};
  for (const name of (await readdir('scripts/bench')).filter(n => n.endsWith('.mjs')).sort()) code[name] = digest(await readFile(`scripts/bench/${name}`));
  return {version:1,date:new Date().toISOString(),revision:git(['rev-parse','HEAD']).trim(),dirty:!!status,
    worktree_sha256:digest(changes.join('\n')),harness:code,model:artifacts,
    addon_sha256:digest(await readFile(`native/${addon[0]}`)),
    machine:{cpu:os.cpus()[0]?.model,logical_cpus:os.cpus().length,memory_bytes:os.totalmem(),os:os.release(),platform:os.platform(),arch:os.arch()},
    node:process.version,rustc:spawnSync('rustc',['--version'],{encoding:'utf8'}).stdout.trim(),
    configuration:{profile:config.profile,backend:'cpu',precision:'float32',threshold:0.5,policy:'offensive_spans'},
    lock_sha256:digest(await readFile('Cargo.lock')),package_lock_sha256:digest(await readFile('package-lock.json'))};
}
export function sameIdentity(start, end) {
  return ['revision','worktree_sha256','harness','model','addon_sha256','machine','configuration','lock_sha256','package_lock_sha256','node','rustc']
    .every(key=>JSON.stringify(start[key])===JSON.stringify(end[key]));
}
export function errorCode(error) {
  return ['invalid_input','invalid_options','invalid_span','protected_span_detected','model_unavailable','model_incompatible','inference_failed','processing_timeout','busy','policy_unavailable'].includes(error?.message) ? error.message : 'benchmark_execution_failed';
}
export function failureCode(error) {
  if (['ENOENT','ENOTDIR'].includes(error?.code)) return 'prerequisite_missing';
  if (error?.code==='EACCES') return 'prerequisite_unreadable';
  if (error?.code==='EEXIST') return 'output_exists';
  const allowed = ['invalid_arguments','invalid_command','output_exists','native_addon_unavailable','git_identity_unavailable',
    'invalid_corpus','invalid_annotation','invalid_annotation_span','annotation_identity_mismatch','annotation_conflict','annotation_policy_version_mismatch','duplicate_or_invalid_id',
    'empty_corpus','split_leakage','missing_reports','incomplete_report','incompatible_reports','incomplete_quality','incomplete_e2e',
    'incomplete_performance','snapshot_evidence_mismatch','snapshot_summary_mismatch','missing_readme_markers'];
  return allowed.includes(error?.message) ? error.message : errorCode(error);
}
