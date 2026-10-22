import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {cp, mkdir, mkdtemp, readFile, rm, writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {pathToFileURL} from 'node:url';
import test from 'node:test';
import {loadModelCatalog} from '../scripts/model-catalog.mjs';

const artifact = 'synthetic detector artifact';

async function fixture(t, cached) {
  const directory = await mkdtemp(join(tmpdir(), 'craig-model-cache-'));
  t.after(() => rm(directory, {recursive: true, force: true}));
  for (const name of ['scripts', 'models', 'downloads']) await mkdir(join(directory, name));
  for (const name of ['fetch-model.mjs', 'model-catalog.mjs']) {
    await cp(new URL(`../scripts/${name}`, import.meta.url), join(directory, 'scripts', name));
  }
  const catalog = await loadModelCatalog();
  catalog.models.find(model => model.id === 'mudes/en-base').artifacts = [{
    file: 'model.bin', sha256: createHash('sha256').update(artifact).digest('hex'),
  }];
  await writeFile(join(directory, 'models/catalog.json'), JSON.stringify(catalog));
  const destination = join(directory, 'downloads/model.bin');
  if (cached !== undefined) await writeFile(destination, cached);
  return {
    destination,
    run(response) {
      const source = `
        process.argv[2] = ${JSON.stringify(join(directory, 'downloads'))};
        let calls = 0;
        globalThis.fetch = async () => {
          calls++;
          if (${response === undefined}) throw Error('unexpected network access');
          return new Response(${JSON.stringify(response ?? '')});
        };
        await import(${JSON.stringify(pathToFileURL(join(directory, 'scripts/fetch-model.mjs')).href)});
        console.log('network calls: ' + calls);
      `;
      return spawnSync(process.execPath, ['--input-type=module', '-e', source], {
        cwd: directory, encoding: 'utf8', timeout: 10000,
      });
    },
  };
}

test('verified model cache works offline and still checks artifact bytes', async t => {
  const cache = await fixture(t, artifact);
  const result = cache.run();
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /network calls: 0/);
  assert.equal(await readFile(cache.destination, 'utf8'), artifact);
});

test('missing and corrupt cached models are fetched and verified', async t => {
  for (const cached of [undefined, 'corrupt cached bytes']) {
    const cache = await fixture(t, cached);
    const result = cache.run(artifact);
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /network calls: 1/);
    assert.equal(await readFile(cache.destination, 'utf8'), artifact);
  }
});

test('invalid replacement downloads cannot become verified cache entries', async t => {
  const cache = await fixture(t, 'corrupt cached bytes');
  const result = cache.run('invalid upstream bytes');
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /checksum mismatch/);
  assert.equal(await readFile(cache.destination, 'utf8'), 'corrupt cached bytes');
});

test('CI verifies restored model bytes before saving or running consumers', async () => {
  const action = await readFile(new URL('../.github/actions/model-cache/action.yml', import.meta.url), 'utf8');
  const steps = action.split('    - name: ');
  const verify = steps.find(step => step.startsWith('verify or download detector'));
  assert.match(verify, /run: node scripts\/fetch-model\.mjs/);
  assert.doesNotMatch(verify, /\bif:/);
  assert(action.indexOf('actions/cache/restore@v4') < action.indexOf(verify));
  assert(action.indexOf(verify) < action.indexOf('actions/cache/save@v4'));
  assert.match(action, /key: \$\{\{ steps\.restore\.outputs\.cache-primary-key \}\}/);
  assert.doesNotMatch(action, /restore-keys:|always\(\)/);
  for (const workflow of ['verify', 'release', 'benchmarks']) {
    const yaml = await readFile(new URL(`../.github/workflows/${workflow}.yml`, import.meta.url), 'utf8');
    assert.match(yaml, /uses: \.\/\.github\/actions\/model-cache/);
    assert.doesNotMatch(yaml, /run: npm run model:fetch/);
  }
});
