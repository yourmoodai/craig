import { createRequire } from 'node:module';
import { readFile, writeFile } from 'node:fs/promises';
import assert from 'node:assert/strict';
const { loadDetector, applyReplacements } = createRequire(import.meta.url)('../native/index.cjs');
const detector = await loadDetector({modelDir: process.env.MIDDLE_FINGER_MODEL_DIR ?? '.cache/models/mudes-en-base'});
const fixtures = JSON.parse(await readFile(new URL('../tests/fixtures/model.json',import.meta.url),'utf8'));
const results = [];
for (const fixture of fixtures) {
  const started = performance.now();
  try {
    const result = await detector.rewrite(fixture.text);
    results.push({id:fixture.id, expected:fixture.expected, actual:result.replacementCount ? 'rewritten':'unchanged', source_preserved:result.text===fixture.expected_text, milliseconds:Math.round(performance.now()-started)});
  } catch (error) {
    results.push({id:fixture.id, expected:fixture.expected, actual:error.message, milliseconds:Math.round(performance.now()-started)});
  }
}
const input = '🙂 You idiot, fix the test.';
const detected = await detector.detect(input);
const replaced = applyReplacements(input, detected.spans);
assert.equal(replaced.text, (await detector.rewrite(input)).text);
const pending = detector.rewrite(input);
assert.throws(()=>detector.rewrite(input), /busy/);
await pending;
const report = {model:'mudes/en-base', revision:'1600298e0458d961c2c749da7a098f69b4be478a', threshold:0.5, results, mismatches:results.filter(r=>r.expected!==r.actual || r.source_preserved===false).length};
console.log(JSON.stringify(report,null,2));
if (process.argv[2]) await writeFile(process.argv[2],JSON.stringify(report,null,2)+'\n');
if (report.mismatches) process.exitCode=1;
