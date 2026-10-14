import { readFile } from 'node:fs/promises';

const hex = /^[a-f0-9]{64}$/;
const roles = new Set(['delivery_detector', 'local_judge', 'cross_model_judge']);

export async function loadModelCatalog() {
  return validateModelCatalog(JSON.parse(await readFile(new URL('../models/catalog.json', import.meta.url), 'utf8')));
}

export function validateModelCatalog(catalog) {
  if (!catalog || catalog.schema_version !== 1 || !Array.isArray(catalog.models)) throw Error('invalid_model_catalog');
  const ids = new Set();
  let detectors = 0;
  for (const model of catalog.models) {
    if (!model || typeof model.id !== 'string' || !model.display_name || !model.publisher || !model.task || ids.has(model.id) || model.license !== 'Apache-2.0'
      || model.distribution !== 'explicit_local_download' || !Array.isArray(model.roles) || !model.roles.length
      || !Array.isArray(model.scope?.allowed) || !model.scope.allowed.length || !Array.isArray(model.scope?.prohibited) || !model.scope.prohibited.length
      || !model.provenance?.upstream_url || !model.provenance?.upstream_revision || !model.provenance?.license_url
      || !Array.isArray(model.artifacts) || !model.artifacts.length) throw Error('invalid_model_catalog');
    ids.add(model.id);
    for (const role of model.roles) if (!roles.has(role)) throw Error('invalid_model_catalog');
    if (!model.scope.prohibited.includes('provider_policy_enforcement') || !model.scope.prohibited.includes('certification')) throw Error('invalid_model_catalog');
    if (model.roles.includes('local_judge') && !model.scope.prohibited.includes('delivery_detector')) throw Error('invalid_model_catalog');
    if (model.roles.includes('delivery_detector')) detectors += 1;
    for (const artifact of model.artifacts) {
      if (typeof artifact.file !== 'string' || !hex.test(artifact.sha256 ?? '')) throw Error('invalid_model_catalog');
    }
  }
  if (detectors !== 1 || !ids.has('mudes/en-base') || !ids.has('Qwen/Qwen3-4B-Instruct-2507') || !ids.has('google/gemma-4-12B-it')) throw Error('invalid_model_catalog');
  return catalog;
}

export function modelById(catalog, id) {
  const model = catalog.models.find((entry) => entry.id === id);
  if (!model) throw Error('model_not_cataloged');
  return model;
}

export function renderModelNotice(catalog) {
  const lines = ['craig model artifacts', ''];
  for (const model of catalog.models) {
    lines.push(`${model.display_name} (${model.id})`,
      `Role: ${model.roles.join(', ')}.`,
      `License: ${model.license}; ${model.provenance.license_url}`,
      `Upstream: ${model.provenance.upstream_url} @ ${model.provenance.upstream_revision}.`);
    if (model.provenance.source_url) lines.push(`Source: ${model.provenance.source_url}.`);
    if (model.provenance.converter) lines.push(`Conversion: ${model.provenance.converter} (${model.provenance.converter_url}) @ ${model.provenance.converter_revision}.`);
    for (const artifact of model.artifacts) lines.push(`Artifact: ${artifact.file}; SHA-256 ${artifact.sha256}${artifact.quantization ? `; ${artifact.quantization}` : ''}.`);
    lines.push('');
  }
  lines.push('Candidate modifications: classifier weights fine-tuned for the policy named in',
    'manifest.json; the pinned BERT encoder and tokenizer remain unchanged. See',
    'manifest.json and training.json for the training corpus fingerprint, parameters,',
    'checkpoint selection, and parity evidence. Candidate artifacts are not promoted',
    'or endorsed by the upstream authors.', '',
    'Models are explicitly downloaded local artifacts and are not included in the npm',
    'package. Synthetic bootstrap examples are offered under CC0-1.0. Other supplied',
    'datasets retain their own licenses. Model licensing does not grant rights in',
    'external data.', '');
  return lines.join('\n');
}
