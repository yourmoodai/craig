import {execFileSync} from 'node:child_process';
import {readFileSync, existsSync} from 'node:fs';
import {fileURLToPath} from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const legacy = /craig|middle[\s_-]*fi(?:ng|gn)er/ig;
const historical_sources = new Set([
  'tests/fixtures/corpus.jsonl',
  'tests/fixtures/decision-training-v2.jsonl',
]);
const files = execFileSync('git', ['ls-files', '-z', '--cached', '--others', '--exclude-standard'], {cwd:root, encoding:'utf8'}).split('\0').filter(Boolean);
const errors = [];
for (const path of new Set(files)) {
  if (!existsSync(new URL(path, new URL('../', import.meta.url)))) continue;
  if (path.match(legacy)) errors.push(`${path}: legacy filename`);
  const bytes = readFileSync(new URL(path, new URL('../', import.meta.url)));
  if (bytes.includes(0)) continue;
  if (path.startsWith('benchmarks/results/') || path === 'docs/migration.md' || path === 'scripts/verify-branding.mjs') continue;
  const content = bytes.toString('utf8');
  for (const [index, line] of content.split('\n').entries()) {
    // Preserve original corpus provenance without exempting prompt text or labels.
    const checked = historical_sources.has(path)
      ? line.replace(/"source"\s*:\s*"Craig synthetic corpus generator v1"/g, '')
      : line;
    if (checked.match(legacy)) errors.push(`${path}:${index+1}: legacy product name`);
  }
}
if (errors.length) {
  console.error(errors.join('\n'));
  process.exitCode = 1;
} else console.log('savior branding verified; historical references preserved');
