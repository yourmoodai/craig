import { readFile, readdir } from 'node:fs/promises';
import { loadModelCatalog, renderModelNotice } from './model-catalog.mjs';

const root = new URL('../', import.meta.url);
const marketplaceKeywords = ['privacy', 'local-inference', 'offline', 'open-source-models', 'span-replacement', 'claude-code-mod'];
const cargoKeywords = ['privacy', 'local-inference', 'offline', 'open-source-models', 'span-replacement'];
const catalog = await loadModelCatalog();
const [notice, packageJson, plugin, marketplace, cargoManifests] = await Promise.all([
  readFile(new URL('models/NOTICE', root), 'utf8'),
  readFile(new URL('package.json', root), 'utf8').then(JSON.parse),
  readFile(new URL('.claude-plugin/plugin.json', root), 'utf8').then(JSON.parse),
  readFile(new URL('.claude-plugin/marketplace.json', root), 'utf8').then(JSON.parse),
  Promise.all((await readdir(new URL('crates/', root), { withFileTypes: true }))
    .filter((entry) => entry.isDirectory())
    .map((entry) => readFile(new URL(`crates/${entry.name}/Cargo.toml`, root), 'utf8'))),
]);
if (notice !== renderModelNotice(catalog)) throw Error('stale_model_notice');
if (JSON.stringify(packageJson.keywords) !== JSON.stringify(marketplaceKeywords) || JSON.stringify(plugin.keywords) !== JSON.stringify(marketplaceKeywords)) throw Error('model_keywords_mismatch');
if (!marketplace.plugins?.some((entry) => entry.name === plugin.name && entry.description.includes('Claude Code mod'))) throw Error('marketplace_mod_metadata_mismatch');
if (!packageJson.files.includes('models')) throw Error('model_catalog_not_packaged');
const cargoKeywordLine = `keywords = [${cargoKeywords.map((keyword) => JSON.stringify(keyword)).join(', ')}]`;
for (const cargo of cargoManifests) {
  for (const required of ['repository.workspace = true', 'homepage.workspace = true', cargoKeywordLine]) {
    if (!cargo.includes(required)) throw Error('cargo_model_metadata_mismatch');
  }
}
console.log(`verified ${catalog.models.length} cataloged local models`);
