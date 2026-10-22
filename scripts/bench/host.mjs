const phases = ['ready','trust_selection','trusted','pane','metrics','consent','submission',
  'metrics_recorded','failure_submission','failure_detected','draft_ready','recovery_submission',
  'provider_received','completed','failed'];
const reasons = ['installed_host_timeout','installed_host_exited','unexpected_authentication',
  'provider_request_observed','original_reached_provider','draft_not_observed','unexpected_consent',
  'ui_worker_mismatch','host_execution_failed','host_wrapper_requires_native_binary',
  'host_prerequisite_missing','host_version_unsupported'];
const finite = value => Number.isFinite(value) && value >= 0;

// Host output is untrusted: only schema-checked, content-free evidence is persisted.
export function hostStream(scenario, strategy, checkpoint = () => {}) {
  const events = [];
  let evidence = null, reason = null, terminal = false;
  const line = text => {
    if (!text.startsWith('{')) return;
    let value;
    try { value = JSON.parse(text); } catch { throw Error('malformed_json'); }
    if (reasons.includes(value.error) && Object.keys(value).length === 1) {
      reason = value.error;
      return;
    }
    if (terminal || value.protocol !== 1 || value.scenario !== scenario || value.strategy !== strategy ||
      !phases.includes(value.event) || !finite(value.elapsed_ms) || value.elapsed_ms < (events.at(-1)?.elapsed_ms ?? 0)) throw Error('schema_mismatch');
    const event = {protocol:1,event:value.event,scenario,strategy,elapsed_ms:value.elapsed_ms};
    if (value.event === 'failure_detected') {
      if (value.no_request_on_failure !== true) throw Error('schema_mismatch');
      event.no_request_on_failure = true;
    }
    if (value.event === 'failed') {
      if (!reasons.includes(value.reason) || !phases.includes(value.phase)) throw Error('schema_mismatch');
      reason = value.reason;
      event.reason = reason;
      event.phase = value.phase;
      for (const key of ['draft_text_visible','prompt_marker_visible','draft_words_visible']) {
        if (typeof value[key] === 'boolean') event[key] = value[key];
      }
      terminal = true;
    }
    if (value.event === 'completed') {
      if (value.host_version !== '2.1.290 (Claude Code)' || !/^[a-f0-9]{64}$/.test(value.host_sha256) ||
        !finite(value.submission_to_provider_ms) || value.samples !== 1 || !events.some(e => e.event === 'provider_received')) throw Error('schema_mismatch');
      const completed = {scenario,strategy,host_version:value.host_version,host_sha256:value.host_sha256,
        submission_to_provider_ms:value.submission_to_provider_ms,samples:1};
      if (scenario !== 'success') {
        const required = ['failure_detected','draft_ready','recovery_submission','provider_received'];
        const indices = required.map(phase => events.findIndex(e => e.event === phase));
        const injection = scenario === 'processing_timeout' ? 'bridge_response' : scenario === 'worker_exit' ? 'bridge_process_exit' : 'none';
        const recovery = scenario === 'protected' ? 'edited_draft' : 'retained_draft_retry';
        if (indices.some((index, n) => index < 0 || n > 0 && index <= indices[n-1]) ||
          value.no_request_on_failure !== true || value.injection !== injection || value.recovery !== recovery) throw Error('schema_mismatch');
        Object.assign(completed,{no_request_on_failure:true,injection,recovery});
      }
      evidence = completed;
      Object.assign(event,evidence);
      terminal = true;
    }
    events.push(event);
    checkpoint(event);
  };
  return {line, finish(result) {
    const passed = result.status === 0 && !result.error && evidence !== null && reason === null;
    return {status:passed?'passed':result.error??'host_failed',evidence,
      reason:reason??(passed?null:result.error??(result.status!==0?'process_exit':'incomplete_host_report')),
      last_stage:events.at(-1)?.phase??events.at(-1)?.event??null,
      no_request_on_failure:events.some(e => e.event === 'failure_detected') ? true : null,
      protocol:1,events,process_ms:result.milliseconds};
  }};
}
