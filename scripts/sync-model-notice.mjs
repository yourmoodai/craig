import { writeFile } from 'node:fs/promises';
import { loadModelCatalog, renderModelNotice } from './model-catalog.mjs';

await writeFile(new URL('../models/NOTICE', import.meta.url), renderModelNotice(await loadModelCatalog()));
