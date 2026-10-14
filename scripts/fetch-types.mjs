import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
const path = new URL('../.cache/host-types/claude-code.d.ts', import.meta.url);
const digest = 'e1c060069af2d494fc884e03f136ed0b3ddaa0eb5341e72e3acd712e5f637c49';
let bytes;
try { bytes = await readFile(path); } catch {}
if (!bytes || createHash('sha256').update(bytes).digest('hex') !== digest) {
  const response = await fetch('https://raw.githubusercontent.com/anthropics/claude-code/3deb873687379f0c199a69fdaccc65f0865f7a29/mods/types/claude-code.d.ts');
  if (!response.ok) throw new Error('type download failed');
  bytes = Buffer.from(await response.arrayBuffer());
  if (createHash('sha256').update(bytes).digest('hex') !== digest) throw new Error('type digest mismatch');
  await mkdir(new URL('.', path), { recursive: true });
  await writeFile(path, bytes);
}
console.log('host type declarations verified');
