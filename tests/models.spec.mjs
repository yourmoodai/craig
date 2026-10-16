import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import test from 'node:test';
import { loadModelCatalog, validateModelCatalog } from '../scripts/model-catalog.mjs';

test('model catalog keeps judges outside the delivery path', async () => {
  const catalog = await loadModelCatalog();
  const detector = catalog.models.filter((model) => model.roles.includes('delivery_detector'));
  const judges = catalog.models.filter((model) => model.roles.includes('local_judge'));
  assert.equal(detector.length, 1);
  assert.equal(detector[0].id, 'mudes/en-base');
  assert.equal(judges.length, 2);
  assert(judges.every((model) => !model.roles.includes('delivery_detector')));
});

test('model catalog rejects missing provenance and unpinned artifacts', async () => {
  const catalog = await loadModelCatalog();
  const missingProvenance = structuredClone(catalog);
  missingProvenance.models[0].provenance.upstream_revision = '';
  assert.throws(() => validateModelCatalog(missingProvenance), /invalid_model_catalog/);
  const invalidDigest = structuredClone(catalog);
  invalidDigest.models[1].artifacts[0].sha256 = '0'.repeat(63);
  assert.throws(() => validateModelCatalog(invalidDigest), /invalid_model_catalog/);
  const missingBoundary = structuredClone(catalog);
  missingBoundary.models[2].scope.prohibited = ['generative_rewrite'];
  assert.throws(() => validateModelCatalog(missingBoundary), /invalid_model_catalog/);
});

test('npm package includes attribution metadata and excludes model weights', () => {
  const result = spawnSync('npm', ['pack', '--dry-run', '--json'], { cwd: new URL('..', import.meta.url), encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  const files = JSON.parse(result.stdout)[0].files.map((entry) => entry.path);
  assert(files.includes('models/catalog.json'));
  assert(files.includes('models/NOTICE'));
  assert(files.every((path) => !path.endsWith('.bin') && !path.endsWith('.gguf')));
});
