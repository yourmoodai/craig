import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
export const root = fileURLToPath(new URL('..', import.meta.url));
export const packaged = existsSync(join(root,'release.json'));
export const target = `${process.platform}-${process.arch}`;
export const workerBinary = packaged
  ? join(root,'runtime',target,`craig-review${process.platform==='win32'?'.exe':''}`)
  : join(root,'target/release',`craig-review${process.platform==='win32'?'.exe':''}`);
