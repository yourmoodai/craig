import { mkdir, readFile, stat, rename } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { resolve } from 'node:path';
import { createWriteStream } from 'node:fs';
import { Readable } from 'node:stream';
import { pipeline } from 'node:stream/promises';
import { loadModelCatalog, modelById } from './model-catalog.mjs';

const model = modelById(await loadModelCatalog(), 'mudes/en-base');
const { upstream_revision: revision, upstream_url } = model.provenance;
const dir = resolve(process.argv[2] ?? '.cache/models/mudes-en-base');
await mkdir(dir, { recursive: true });

function digest(bytes) {
  return createHash('sha256').update(bytes).digest('hex');
}

for (const { file: name, sha256: expected } of model.artifacts) {
  const destination = resolve(dir, name);
  try {
    if (digest(await readFile(destination)) === expected) {
      console.log(`verified ${name}`);
      continue;
    }
  } catch {}
  const temporary = destination + '.partial';
  let offset = 0;
  try { offset = (await stat(temporary)).size; } catch {}
  const response = await fetch(`${upstream_url}/resolve/${revision}/${name}?download=true&offset=${offset}`, {
    headers: offset ? { Range: `bytes=${offset}-` } : {}, signal: AbortSignal.timeout(1200000),
  });
  if (!response.ok || !response.body) throw Error(`download failed: ${name}`);
  if (response.status === 206 && !response.headers.get('content-range')?.startsWith(`bytes ${offset}-`)) throw Error(`invalid download range: ${name}`);
  await pipeline(Readable.fromWeb(response.body), createWriteStream(temporary, { mode: 0o600, flags: response.status === 206 ? 'a' : 'w' }));
  if (digest(await readFile(temporary)) !== expected) throw Error(`checksum mismatch: ${name}`);
  await rename(temporary, destination);
  console.log(`verified ${name}`);
}
console.log(`model directory: ${dir}`);
