import { mkdir, readFile, stat, rename } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { resolve } from 'node:path';
import { createWriteStream } from 'node:fs';
import { Readable } from 'node:stream';
import { pipeline } from 'node:stream/promises';
const revision = '1600298e0458d961c2c749da7a098f69b4be478a';
const files = {
  'config.json': 'a096962192bd2a13b3c83e577cf78b620f38fc7d',
  'model_args.json': '9195cc734d0cc029e7dfc5cacbea8c3092bc158e',
  'tokenizer_config.json': 'd02f35d7b99af585ba7d2cd7857bda5b8f331d3a',
  'vocab.txt': '2ea941cc79a6f3d7985ca6991ef4f67dad62af04',
  'pytorch_model.bin': 'a5b81cea46c128250dd422c1ddab7506a5ba46946708f3cc74c234b1dbe38b4b',
};
const dir = resolve(process.argv[2] ?? '.cache/models/mudes-en-base');
await mkdir(dir, { recursive: true });
function digest(name, bytes) {
  const hash = createHash(name.endsWith('.bin') ? 'sha256' : 'sha1');
  if (!name.endsWith('.bin')) hash.update(`blob ${bytes.length}\0`);
  return hash.update(bytes).digest('hex');
}
for (const [name, expected] of Object.entries(files)) {
  const destination = resolve(dir, name);
  try { if (digest(name, await readFile(destination)) === expected) { console.log(`verified ${name}`); continue; } } catch {}
  const temporary = destination + '.partial';
  let offset = 0;
  try { offset = (await stat(temporary)).size; } catch {}
  const response = await fetch(`https://huggingface.co/mudes/en-base/resolve/${revision}/${name}?download=true&offset=${offset}`, {
    headers: offset ? { Range: `bytes=${offset}-` } : {}, signal: AbortSignal.timeout(1200000),
  });
  if (!response.ok || !response.body) throw Error(`download failed: ${name}`);
  if (response.status === 206 && !response.headers.get('content-range')?.startsWith(`bytes ${offset}-`)) throw Error(`invalid download range: ${name}`);
  await pipeline(Readable.fromWeb(response.body), createWriteStream(temporary, { mode: 0o600, flags: response.status === 206 ? 'a' : 'w' }));
  if (digest(name, await readFile(temporary)) !== expected) throw Error(`checksum mismatch: ${name}`);
  await rename(temporary, destination);
  console.log(`verified ${name}`);
}
console.log(`model directory: ${dir}`);
