#!/usr/bin/env node
import { createRequire } from 'node:module';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
const require = createRequire(import.meta.url);
const max_bytes = 65_536; // 64 KiB
const allowed_errors = new Set(['invalid_input','invalid_span','invalid_options','protected_span_detected','model_unavailable','model_incompatible','inference_failed','processing_timeout','busy','invalid_request','invalid_command','model_not_configured','policy_unavailable']);

async function main() {
  const command = process.argv[2];
  if (!command || command === '--help') { process.stdout.write('savior rewrite | doctor | inspect | traces status|enable|disable|export|delete\n'); return; }
  const root=fileURLToPath(new URL('..',import.meta.url));
  if (['inspect','traces'].includes(command)) {
    if (existsSync(join(root,'release.json')) && !process.env.SAVIOR_RUNTIME_DIR) {
      const {prepare_runtime}=await import('./bootstrap.mjs');
      process.env.SAVIOR_RUNTIME_DIR=(await prepare_runtime()).runtime_dir;
    }
    const binary=join(process.env.SAVIOR_RUNTIME_DIR || join(root,'target/release'),'savior-review'+(process.platform==='win32'?'.exe':''));
    const child=spawnSync(binary,process.argv.slice(2),{stdio:'inherit'});
    process.exitCode=child.status ?? 1; return;
  }
  if (!['rewrite', 'doctor'].includes(command) || process.argv.length !== 3) throw Error('invalid_command');
  const modelDir = process.env.SAVIOR_MODEL_DIR;
  if (!modelDir) throw Error('model_not_configured');
  const threshold = process.env.SAVIOR_THRESHOLD === undefined ? undefined : Number(process.env.SAVIOR_THRESHOLD);
  const policy = process.env.SAVIOR_POLICY ?? 'offensive_spans';
  const replacementOptions = {
    strategy: process.env.SAVIOR_REPLACEMENT_STRATEGY ?? 'fixed_phrase',
    ...(process.env.SAVIOR_REPLACEMENT === undefined ? {} : {marker: process.env.SAVIOR_REPLACEMENT}),
  };
  let text; let assess = false; let trace = false;
  if (command === 'rewrite') {
    const chunks = []; let size = 0;
    for await (const chunk of process.stdin) {
      size += chunk.length;
      if (size > max_bytes * 8) throw Error('invalid_request');
      chunks.push(chunk);
    }
    let request;
    try { request = JSON.parse(Buffer.concat(chunks).toString('utf8')); } catch { throw Error('invalid_request'); }
    if (!request || ![1,2,3].includes(request.version) || typeof request.text !== 'string' || Object.keys(request).some(key => !['version','text','assess'].includes(key))) throw Error('invalid_request');
    if (request.version === 1 && request.assess !== undefined || request.assess !== undefined && typeof request.assess !== 'boolean') throw Error('invalid_request');
    text = request.text;
    assess = request.version >= 2 && request.assess === true;
    trace = request.version === 3;
  }
  const { loadDetector } = process.env.SAVIOR_RUNTIME_DIR ? require(join(process.env.SAVIOR_RUNTIME_DIR,'savior.node')) : require('../native/index.cjs');
  const detector = await loadDetector({ modelDir, threshold, policy, detector: process.env.SAVIOR_DETECTOR ?? 'bert' });
  if (command === 'doctor') { process.stdout.write(JSON.stringify({ version:1, status:'ready' }) + '\n'); return; }
  if (trace) { process.stdout.write(await detector.rewriteTraced(text,replacementOptions,assess)+'\n'); return; }
  if (assess) {
    const result = await detector.assess(text, replacementOptions);
    const a = result.assessment;
    const hasText = result.text !== null && result.text !== undefined;
    const assessment = {input_characters:a.inputCharacters, removed_characters:a.removedCharacters, replacement_count:a.replacementCount, removed_proportion:a.removedProportion, detector_confidence:a.detectorConfidence ?? null, residual_confidence:a.residualConfidence ?? null, protected_abstention:a.protectedAbstention};
    process.stdout.write(JSON.stringify({version:2,status:hasText ? (a.replacementCount ? 'rewritten':'unchanged') : 'error', text:result.text, code:hasText ? undefined : 'protected_span_detected', assessment})+'\n');
    return;
  }
  const result = await detector.rewrite(text, replacementOptions);
  process.stdout.write(JSON.stringify({ version:1, status:result.replacementCount ? 'rewritten':'unchanged', text:result.text, replacement_count:result.replacementCount }) + '\n');
}
main().catch(error => {
  const code = allowed_errors.has(error?.message) ? error.message : 'bridge_failed';
  process.stdout.write(JSON.stringify({ version:1, status:'error', code }) + '\n');
  if (process.env.LOG_LEVEL !== 'silent') process.stderr.write(`savior: ${code}\n`);
  process.exitCode = 1;
});
