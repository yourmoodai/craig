import {createRequire} from 'node:module';
import {readFile, appendFile} from 'node:fs/promises';
import {json, digest, validate, select, save, applyAnnotations, annotationFor, annotationPolicies} from './corpus.mjs';
import {identity, sameIdentity, strategies, strategyName, errorCode} from './common.mjs';

const ratio = (n,d) => d ? n/d : null;
export function classification(rows) {
  const counts = {examples:rows.length,scored:0,true_positive:0,true_negative:0,false_positive:0,false_negative:0,abstentions:0,errors:0};
  for (const r of rows) {
    if (r.predicted === null) { counts[r.detection_status==='protected_span_detected'?'abstentions':'errors']++; continue; }
    counts.scored++;
    counts[r.expected ? (r.predicted?'true_positive':'false_negative') : (r.predicted?'false_positive':'true_negative')]++;
  }
  const c = counts;
  return {...c,accuracy:ratio(c.true_positive+c.true_negative,c.scored),precision:ratio(c.true_positive,c.true_positive+c.false_positive),
    recall:ratio(c.true_positive,c.true_positive+c.false_negative),f1:ratio(2*c.true_positive,2*c.true_positive+c.false_positive+c.false_negative),
    false_positive_rate:ratio(c.false_positive,c.false_positive+c.true_negative)};
}
export function preservation(text, result, replacement) {
  let cursor = 0, outputCursor = 0;
  for (const span of result.spans) {
    if (!Number.isInteger(span.start) || !Number.isInteger(span.end) || span.start < cursor || span.end <= span.start || span.end > text.length) return false;
    const gap = text.slice(cursor,span.start);
    if (result.text.slice(outputCursor,outputCursor+gap.length) !== gap) return false;
    outputCursor += gap.length;
    const marker = replacement.strategy === 'omit' ? '' : replacement.marker ?? 'ya ya ya';
    if (result.text.slice(outputCursor,outputCursor+marker.length) !== marker) return false;
    outputCursor += marker.length; cursor = span.end;
  }
  return result.replacementCount === result.spans.length && result.text.slice(outputCursor) === text.slice(cursor);
}
export function spanCounts(expected, actual) {
  const gold = new Set(expected.map(s => `${s.start}:${s.end}`)), predicted = new Set(actual.map(s => `${s.start}:${s.end}`));
  const units = spans => { const set = new Set(); for (const {start,end} of spans) for(let i=start;i<end;i++) set.add(i); return set; };
  const e = units(expected), a = units(actual);
  return {true_positive:[...predicted].filter(s=>gold.has(s)).length,false_positive:[...predicted].filter(s=>!gold.has(s)).length,
    false_negative:[...gold].filter(s=>!predicted.has(s)).length,annotated_units:e.size,removed_annotated_units:[...e].filter(i=>a.has(i)).length,
    collateral_units:[...a].filter(i=>!e.has(i)).length};
}
export function replacementSummary(rows, policy='offensive_spans') {
  const summary = {replacement:{},reviewed_spans:{}};
  for (const replacement of strategies) {
    const name = strategyName(replacement), values = rows.map(r=>r.strategies[name]).filter(Boolean);
    const successful = values.filter(v=>['rewritten','unchanged'].includes(v.status));
    summary.replacement[name] = {attempted:values.length,successful:successful.length,
      preservation_passed:successful.filter(r=>r.preserved).length,parity_passed:successful.filter(r=>r.parity).length,
      abstentions:values.filter(v=>v.status==='protected_span_detected').length,
      abstention_parity_passed:values.filter(v=>v.status==='protected_span_detected' && v.parity).length,
      errors:values.filter(v=>!['rewritten','unchanged','protected_span_detected'].includes(v.status)).length};
    const annotated = successful.map(v=>v.annotations?.[policy]??(policy==='offensive_spans'?v.annotation:null)).filter(Boolean);
    const totals = Object.fromEntries(['true_positive','false_positive','false_negative','annotated_units','removed_annotated_units','collateral_units'].map(k=>[k,annotated.reduce((n,a)=>n+a[k],0)]));
    summary.reviewed_spans[name] = {examples:annotated.length,status:annotated.length?'measured':'unavailable',...totals,
      precision:ratio(totals.true_positive,totals.true_positive+totals.false_positive),recall:ratio(totals.true_positive,totals.true_positive+totals.false_negative),
      removal_coverage:ratio(totals.removed_annotated_units,totals.annotated_units),exact_outcomes:annotated.filter(a=>a.exact).length};
  }
  return summary;
}
export function reviewedPolicySummary(rows) {
  return Object.fromEntries(annotationPolicies.map(policy=>[policy,{
    reviewed_cases:rows.filter(r=>r.reviewed_policies?.includes(policy)).length,
    abstentions:rows.filter(r=>r.reviewed_policies?.includes(policy) && r.detection_status==='protected_span_detected').length,
    ...replacementSummary(rows,policy).reviewed_spans
  }]));
}
export function annotationScore(text, actual, annotation, replacement) {
  const expected=annotation.spans.map(([start,end])=>({start,end}));
  let cursor=0, output='';
  const marker=replacement.strategy==='omit'?'':replacement.marker??'ya ya ya';
  for(const span of expected) { output+=text.slice(cursor,span.start)+marker; cursor=span.end; }
  output+=text.slice(cursor);
  return {...spanCounts(expected,actual.spans),exact:output===actual.text};
}
export async function quality(config) {
  const binding = createRequire(import.meta.url)('../../native/index.cjs');
  const corpusBytes = await readFile(config.corpus), corpus = JSON.parse(corpusBytes);
  validate(corpus.rows);
  if (config.annotations) {
    applyAnnotations(corpus.rows,await json(config.annotations));
  }
  const eligible = corpus.rows.filter(r => r.language === 'en');
  const selected = ['scunthorpe','hatecheck'].flatMap(dataset => select(eligible.filter(r=>r.dataset===dataset),config.limit));
  if (!selected.length) throw Error('empty_corpus');
  const report = {version:1,kind:'quality',identity:await identity(config),corpus_sha256:digest(corpusBytes),
    annotations_sha256:config.annotations?digest(await readFile(config.annotations)):null,
    annotation_policy_versions:Object.fromEntries(annotationPolicies.map(p=>[p,[...new Set(corpus.rows.map(r=>annotationFor(r,p)).filter(a=>a?.reviewed).map(a=>a.policy_version??'legacy_unversioned'))]])),
    selection:{method:'category_label_round_robin_sha256_id',limit_per_dataset:config.limit,language:'en',eligible:eligible.length,selected:selected.length,full:config.limit===0},
    status:'running',rows:[],tracks:{},replacement:{},reviewed_spans:{},errors:[]};
  await save(`${config.output}.identity.json`,{...report,rows:undefined});
  const started = performance.now();
  try {
    const detector = await binding.loadDetector({modelDir:config.model,policy:'offensive_spans',threshold:0.5,backend:'cpu'});
    report.load_ms = performance.now()-started;
    for (const [i,row] of selected.entries()) {
      if (i % 25 === 0) process.stderr.write(`quality ${i}/${selected.length}\n`);
      const outcome = {id:row.id,dataset:row.dataset,category:row.category,track:row.track,expected:row.upstream_label,predicted:null,
        reviewed_policies:annotationPolicies.filter(p=>annotationFor(row,p)?.reviewed),strategies:{}};
      try {
        const detection = await detector.detect(row.text);
        const spans = detection.spans.filter(s => s.score >= 0.5);
        outcome.predicted = spans.length > 0;
        for (const replacement of strategies) {
          const key = strategyName(replacement);
          try {
            const result = binding.applyReplacements(row.text,spans,replacement);
            const actual = await detector.rewrite(row.text,replacement);
            const record = {status:result.replacementCount?'rewritten':'unchanged',preserved:preservation(row.text,actual,replacement),
              parity:result.text===actual.text && JSON.stringify(result.spans)===JSON.stringify(actual.spans)};
            for (const policy of outcome.reviewed_policies) {
              record.annotations??={};
              record.annotations[policy]=annotationScore(row.text,actual,annotationFor(row,policy),replacement);
            }
            outcome.strategies[key] = record;
          } catch (error) {
            const status = errorCode(error);
            let parity = false;
            if (status==='protected_span_detected') {
              try { await detector.rewrite(row.text,replacement); }
              catch (actual) { parity=errorCode(actual)===status; }
            }
            outcome.strategies[key] = {status,parity};
          }
        }
      } catch (error) {
        const status = errorCode(error);
        if (status==='protected_span_detected') {
          outcome.detection_status = status;
          for (const replacement of strategies) {
            let parity = false;
            try { await detector.rewrite(row.text,replacement); }
            catch (actual) { parity=errorCode(actual)===status; }
            outcome.strategies[strategyName(replacement)] = {status,parity};
          }
        } else outcome.error = status;
      }
      report.rows.push(outcome);
      await appendFile(`${config.output}.progress.jsonl`,JSON.stringify(outcome)+'\n',{mode:0o600});
    }
  } catch (error) { report.errors.push(errorCode(error)); }
  for (const track of ['profanity','targeted_abuse']) {
    const rows = report.rows.filter(r=>r.track===track);
    report.tracks[track] = {upstream_classification:classification(rows),...replacementSummary(rows),reviewed_policies:reviewedPolicySummary(rows),categories:{}};
    for(const category of new Set(rows.map(r=>r.category))) report.tracks[track].categories[category] = classification(rows.filter(r=>r.category===category));
  }
  Object.assign(report,replacementSummary(report.rows));
  report.reviewed_policies=reviewedPolicySummary(report.rows);
  report.identity_end=await identity(config);
  report.identity_stable=sameIdentity(report.identity,report.identity_end) && report.corpus_sha256===digest(await readFile(config.corpus)) &&
    report.annotations_sha256===(config.annotations?digest(await readFile(config.annotations)):null);
  report.status = !report.identity_stable || report.errors.length || report.rows.length !== selected.length || report.rows.some(r=>r.error) ||
    Object.values(report.replacement).some(r=>r.errors || r.preservation_passed!==r.successful || r.parity_passed!==r.successful || r.abstention_parity_passed!==r.abstentions) ? 'failed' : 'completed';
  await save(config.output,report);
  return report;
}
