import { loadDetector, applyReplacements, type RewriteResult } from 'craig';
async function check(): Promise<RewriteResult> {
 const detector = await loadDetector({ modelDir: '/local/model', threshold: 0.5, policy: 'offensive_spans', backend: 'core_ml', precision: 'float32', windowBatch: 1, coremlDir: '/local/export', computeUnits: 'cpu_gpu' });
 const found = await detector.detect('text');
 applyReplacements('text', found.spans, { marker: 'ya ya ya' });
 return detector.rewrite('text');
}
void check;
