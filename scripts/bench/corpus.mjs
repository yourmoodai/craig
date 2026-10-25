import {createHash} from 'node:crypto';
import {mkdir, readFile, writeFile} from 'node:fs/promises';
import {resolve, dirname} from 'node:path';
import {pathToFileURL} from 'node:url';

export const digest = value => createHash('sha256').update(value).digest('hex');
export const json = async path => JSON.parse(await readFile(path, 'utf8'));
export async function save(path, value) {
  await mkdir(dirname(path), {recursive:true});
  await writeFile(path, JSON.stringify(value, null, 2) + '\n', {flag:'wx', mode:0o600});
}
export function csv(text) {
  const rows = []; let row = [], field = '', quoted = false, closed = false;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (quoted) {
      if (c === '"' && text[i + 1] === '"') { field += '"'; i++; }
      else if (c === '"') { quoted = false; closed = true; }
      else field += c;
    } else if (c === '"' && !field && !closed) quoted = true;
    else if (c === ',' || c === '\n' || c === '\r') {
      row.push(field); field = ''; closed = false;
      if (c !== ',') {
        if (c === '\r' && text[i + 1] === '\n') i++;
        rows.push(row); row = [];
      }
    } else {
      if (closed || c === '"') throw Error('invalid_csv');
      field += c;
    }
  }
  if (quoted) throw Error('invalid_csv');
  if (field || row.length || closed) rows.push([...row, field]);
  const header = rows.shift();
  if (!header || new Set(header).size !== header.length) throw Error('invalid_csv_header');
  return rows.map(values => {
    if (values.length !== header.length) throw Error('invalid_csv_width');
    return Object.fromEntries(header.map((key, i) => [key, values[i]]));
  });
}
export function normalizeLicense(value) {
  const allowed = {'CC BY 4.0':'CC-BY-4.0', 'CC0 1.0':'CC0-1.0', 'CC-BY-4.0':'CC-BY-4.0', 'CC0-1.0':'CC0-1.0'};
  return typeof value === 'string' && Object.hasOwn(allowed,value) ? allowed[value] : null;
}
export function proposal(text, phrase) {
  if (!phrase || phrase === 'None' || phrase === 'nan') return null;
  const start = text.indexOf(phrase);
  if (start < 0 || text.indexOf(phrase, start + 1) >= 0) return null;
  return {unit:'utf16', spans:[[start,start + phrase.length]], reviewed:false, origin:'upstream_focus_words'};
}
export function normalize(source, data) {
  if (!['scunthorpe','hatecheck'].includes(source.id)) throw Error('unsupported_source');
  const rows = source.id === 'scunthorpe' ? data.trim().split('\n').map(JSON.parse) : csv(data);
  const accepted = [], excluded = [];
  for (const row of rows) {
    const scunthorpe = source.id === 'scunthorpe';
    const id = `${source.id}-${scunthorpe ? row.id : row.case_id}`;
    const license = normalizeLicense(scunthorpe ? row.license : 'CC-BY-4.0');
    if (!license) { excluded.push({id,reason:'unsupported_license'}); continue; }
    const text = scunthorpe ? row.text : row.test_case;
    const flag = scunthorpe ? row.expected?.flagged : {hateful:true,'non-hateful':false}[row.label_gold];
    if (typeof text !== 'string' || !text || typeof flag !== 'boolean') throw Error('invalid_upstream_record');
    if (Buffer.byteLength(text) > 65536) { excluded.push({id,reason:'input_too_large'}); continue; } // 64 KiB
    accepted.push({id, dataset:source.id, upstream_id:String(scunthorpe ? row.id : row.case_id),
      family:scunthorpe ? `scunthorpe-${row.kind}` : `hatecheck-${row.templ_id}`,
      split:'test', category:scunthorpe ? row.kind : row.functionality, language:scunthorpe ? row.lang : 'en',
      track:scunthorpe ? 'profanity' : 'targeted_abuse', text, source:scunthorpe ? row.source : source.repository,
      license, upstream_label:flag, upstream_label_name:scunthorpe ? 'flagged' : 'hateful',
      upstream_metadata:scunthorpe ? row.expected : {target:row.target_ident, reference_template:row.ref_templ_id},
      annotation:scunthorpe ? null : proposal(text, row.focus_words)});
  }
  return {accepted,excluded};
}
function boundary(text, n) {
  return Number.isInteger(n) && n >= 0 && n <= text.length &&
    !(n > 0 && n < text.length && /[\uD800-\uDBFF]/.test(text[n-1]) && /[\uDC00-\uDFFF]/.test(text[n]));
}
export const annotationPolicies = ['offensive_spans','directed_abuse'];
export const classifications = ['policy_mismatch','detection_false_positive','detection_false_negative','span_boundary_error',
  'expected_abstention','unexpected_abstention','unresolved','aligned'];
export function annotationFor(row, policy) {
  return row.annotations?.[policy] ?? (row.annotation?.policy===policy?row.annotation:null);
}
function validateAnnotation(text, a, policy) {
  if (!a || a.unit !== 'utf16' || typeof a.reviewed !== 'boolean' || !a.origin || !Array.isArray(a.spans) ||
    policy && (a.policy!==policy || typeof a.policy_version!=='string' || !a.policy_version.trim()) ||
    a.review_status!==undefined && !['reviewed','proposed','unresolved'].includes(a.review_status) ||
    a.review_status!==undefined && (a.review_status==='reviewed')!==a.reviewed ||
    a.classification!==undefined && !classifications.includes(a.classification) ||
    a.reviewed && (typeof a.reviewer !== 'string' || !a.reviewer.trim() || typeof a.reviewed_at !== 'string' || !Number.isFinite(Date.parse(a.reviewed_at)) || !annotationPolicies.includes(a.policy) || a.classification==='unresolved')) throw Error('invalid_annotation');
  let end = 0;
  for (const span of a.spans) {
    if (!Array.isArray(span) || span.length !== 2 || !span.every(n => boundary(text,n)) || span[0] < end || span[0] >= span[1]) throw Error('invalid_annotation_span');
    end = span[1];
  }
}
export function validate(rows) {
  if (!Array.isArray(rows) || !rows.length) throw Error('empty_corpus');
  const ids = new Set(), families = new Map(), texts = new Map(), versions = new Map();
  for (const row of rows) {
    if (!/^[a-zA-Z0-9_-]+$/.test(row.id) || ids.has(row.id)) throw Error('duplicate_or_invalid_id');
    ids.add(row.id);
    if (!normalizeLicense(row.license) || !row.source?.trim() || typeof row.text !== 'string' || !row.text ||
        !row.text.isWellFormed() || Buffer.byteLength(row.text) > 65536 || typeof row.upstream_label !== 'boolean' ||
        !['scunthorpe','hatecheck'].includes(row.dataset) || typeof row.upstream_id !== 'string' || !row.upstream_id ||
        typeof row.language !== 'string' || !row.language ||
        row.track !== (row.dataset==='scunthorpe'?'profanity':'targeted_abuse') || !row.family || row.split !== 'test') throw Error('invalid_corpus');
    for (const [map,key] of [[families,row.family],[texts,digest(row.text)]]) {
      if (map.has(key) && map.get(key) !== row.split) throw Error('split_leakage');
      map.set(key,row.split);
    }
    if (row.annotation !== null) {
      validateAnnotation(row.text,row.annotation);
    }
    if (row.annotations!==undefined) {
      if (!row.annotations || typeof row.annotations!=='object' || Array.isArray(row.annotations)) throw Error('invalid_annotation');
      for (const [policy,a] of Object.entries(row.annotations)) {
        if(!annotationPolicies.includes(policy)) throw Error('invalid_annotation');
        validateAnnotation(row.text,a,policy);
        if(a.reviewed) {
          if(versions.has(policy) && versions.get(policy)!==a.policy_version) throw Error('annotation_policy_version_mismatch');
          versions.set(policy,a.policy_version);
        }
      }
    }
  }
  return {examples:rows.length, reviewed_spans:rows.filter(r => r.annotation?.reviewed || annotationPolicies.some(p=>r.annotations?.[p]?.reviewed)).length,
    proposed_spans:rows.filter(r => r.annotation && !r.annotation.reviewed).length,
    unique_texts:texts.size, duplicate_texts:rows.length-texts.size};
}
export async function fetchCorpus(directory, manifestPath = 'benchmarks/sources.json') {
  const manifest = await json(manifestPath), accepted = [], excluded = [];
  for (const source of manifest.sources) {
    if (!/^[a-f0-9]{40}$/.test(source.revision)) throw Error('unpinned_source');
    for (const file of source.files) {
      const path = resolve(directory,source.id,file.path);
      let bytes;
      try { bytes = await readFile(path); } catch (error) {
        if (error.code !== 'ENOENT') throw error;
        const response = await fetch(`https://raw.githubusercontent.com/${source.repository}/${source.revision}/${file.path}`, {signal:AbortSignal.timeout(60000)});
        if (!response.ok) throw Error('corpus_download_failed');
        bytes = Buffer.from(await response.arrayBuffer());
        if (digest(bytes) !== file.sha256) throw Error('checksum_mismatch');
        await mkdir(dirname(path),{recursive:true});
        await writeFile(path,bytes,{flag:'wx',mode:0o600});
      }
      if (digest(bytes) !== file.sha256) throw Error('checksum_mismatch');
      if (file.path.endsWith('.jsonl') || file.path.endsWith('.csv')) {
        const result = normalize(source,bytes.toString('utf8'));
        accepted.push(...result.accepted); excluded.push(...result.excluded);
      }
    }
  }
  const result = {version:1,manifest_sha256:digest(await readFile(manifestPath)),...validate(accepted),excluded,rows:accepted};
  const path = resolve(directory,'corpus.json');
  try { const old = await json(path); if (digest(JSON.stringify(old)) !== digest(JSON.stringify(result))) throw Error('corpus_cache_conflict'); }
  catch (error) { if (error.code !== 'ENOENT') throw error; await save(path,result); }
  return {path,examples:result.examples,excluded:excluded.length,proposed_spans:result.proposed_spans};
}
export function applyAnnotations(rows, annotations) {
  if (!Array.isArray(annotations)) throw Error('invalid_annotation');
  const ids = new Set(), proposed=structuredClone(rows), byId=new Map(proposed.map(r=>[r.id,r]));
  for (const entry of annotations) {
    const row = byId.get(entry.id), policy=entry.annotation?.policy, key=`${entry.id}:${policy}`;
    if (!row || ids.has(key) || entry.text_sha256 !== digest(row.text)) throw Error('annotation_identity_mismatch');
    if(!annotationPolicies.includes(policy)) throw Error('invalid_annotation');
    validateAnnotation(row.text,entry.annotation,policy);
    ids.add(key);
    row.annotations??={};
    if(row.annotations[policy] && JSON.stringify(row.annotations[policy])!==JSON.stringify(entry.annotation)) throw Error('annotation_conflict');
    row.annotations[policy]=entry.annotation;
  }
  const result=validate(proposed);
  for(let i=0;i<rows.length;i++) if(proposed[i].annotations) rows[i].annotations=proposed[i].annotations;
  return result;
}
export function select(rows, limit) {
  if (limit === 0) return [...rows];
  if (!Number.isInteger(limit) || limit < 0) throw Error('invalid_limit');
  const buckets = new Map();
  for (const row of rows) {
    const key = `${row.dataset}:${row.category}:${row.language}:${row.upstream_label}`;
    if (!buckets.has(key)) buckets.set(key,[]);
    buckets.get(key).push(row);
  }
  const ordered = [...buckets].sort(([a],[b]) => a.localeCompare(b)).map(([,rows]) => rows.sort((a,b) => digest(a.id).localeCompare(digest(b.id))));
  const result = [];
  for (let round = 0; result.length < limit; round++) {
    let added = false;
    for (const bucket of ordered) if (bucket[round] && result.length < limit) { result.push(bucket[round]); added = true; }
    if (!added) break;
  }
  return result;
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const [command, path = '.cache/bench-corpus', overlay] = process.argv.slice(2);
    if (command === 'fetch') console.log(JSON.stringify(await fetchCorpus(path)));
    else if (command === 'validate') {
      const rows = (await json(resolve(path,'corpus.json'))).rows;
      console.log(JSON.stringify(overlay ? applyAnnotations(rows,await json(overlay)) : validate(rows)));
    }
    else throw Error('invalid_command');
  } catch (error) {
    const codes = ['invalid_command','checksum_mismatch','corpus_cache_conflict','invalid_corpus','invalid_annotation','invalid_annotation_span','annotation_identity_mismatch','duplicate_or_invalid_id','corpus_download_failed'];
    console.error(codes.includes(error.message)?error.message:'corpus_failed'); process.exitCode = 1;
  }
}
