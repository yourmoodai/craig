export const protocol_version = 1;
export const max_text_bytes = 65_536; // 64 KiB
export const max_output_bytes = max_text_bytes * 80 * 4; // 20 MiB: at most 80 Unicode scalars per replaced byte.

export type RewriteRequest = { version: 1; text: string };
export type LanguageAssessment = { input_characters:number; removed_characters:number; replacement_count:number; removed_proportion:number; detector_confidence:number|null; residual_confidence:number|null; protected_abstention:boolean };
export type RewriteResult =
  | { version: 1|2; status: 'rewritten' | 'unchanged'; text: string; assessment?:LanguageAssessment }
  | { version: 1|2; status: 'error'; code: string; assessment?:LanguageAssessment };

export function read_result(value: unknown): RewriteResult {
  if (!value || typeof value !== 'object') throw new Error('invalid_response');
  const result = value as Record<string, unknown>;
  if (result.version !== protocol_version && result.version !== 2) throw new Error('invalid_response');
  let assessment: LanguageAssessment | undefined;
  if (result.version === 2 && result.assessment !== undefined) {
    const a=result.assessment as LanguageAssessment;
    if (!a || ![a.input_characters,a.removed_characters,a.replacement_count].every(n=>Number.isSafeInteger(n)&&n>=0) || a.removed_characters>a.input_characters || !Number.isFinite(a.removed_proportion) || a.removed_proportion<0 || a.removed_proportion>1 || typeof a.protected_abstention!=='boolean' || ![a.detector_confidence,a.residual_confidence].every(n=>n===null || typeof n==='number'&&Number.isFinite(n)&&n>=0&&n<=1)) throw new Error('invalid_response');
    assessment=a;
  }
  if (result.status === 'error' && typeof result.code === 'string' && /^[a-z_]+$/.test(result.code)) {
    return { version: result.version, status: 'error', code: result.code, ...(assessment?{assessment}:{}) };
  }
  if ((result.status === 'rewritten' || result.status === 'unchanged') && typeof result.text === 'string' && result.text.trim()) {
    return { version: result.version, status: result.status, text: result.text, ...(assessment?{assessment}:{}) };
  }
  throw new Error('invalid_response');
}
