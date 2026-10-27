import type { On } from 'claude-code';
import { read_result, max_text_bytes, max_output_bytes } from '../src/protocol';
import { PersonalReview } from './review';
import { Runtime } from './runtime';

export function register(on: On) {
  const review = new PersonalReview();
  const runtime = new Runtime();
  let latest: { original: string; replacement: string } | null = null;
  let generation = 0;
  let submitted = 0;
  let trace_warning = '';
  on('session.start', async ($, e, next) => {
    const host = {
      run: (request: unknown) => $.process.run([runtime.node, $.plugin.root + '/bridge/review.mjs'], {env: runtime.env, stdin: JSON.stringify(request), timeoutMs: (request as {command?: string}).command === 'scan' ? 600000 : 15000}),
      session: () => $.session.id(), now: () => $.clock.now(),
      enabled: () => $.store.get('personal_review_enabled'),
      save: (enabled: boolean) => $.store.set('personal_review_enabled', enabled),
      invalidate: () => { $.ui.invalidate('ui.render'); },
    };
    latest = null; generation += 1;
    await runtime.start({plugin:{root:$.plugin.root},process:{run:(argv,init)=>$.process.run(argv,init)}});
    if (runtime.ready) await review.start(host);
    await $.command.register({ name: 'savior', description: 'inspect the latest local replacement', immediate: true });
    return next(e);
  });
  on('session.end', async ($, e, next) => {
    const host = {
      run: (request: unknown) => $.process.run([runtime.node, $.plugin.root + '/bridge/review.mjs'], {env: runtime.env, stdin: JSON.stringify(request), timeoutMs: (request as {command?: string}).command === 'scan' ? 600000 : 15000}),
      session: () => $.session.id(), now: () => $.clock.now(),
      enabled: () => $.store.get('personal_review_enabled'),
      save: (enabled: boolean) => $.store.set('personal_review_enabled', enabled),
      invalidate: () => { $.ui.invalidate('ui.render'); },
    };
    latest = null; generation += 1;
    await review.end(host);
    return next(e);
  });
  on('command.run', { command: 'savior' }, async ($) => {
    await $.ui.open({ id: 'savior', title: 'local replacement', focus: true, closeOnEscape: true });
    if (!runtime.ready) { await runtime.start({plugin:{root:$.plugin.root},process:{run:(argv,init)=>$.process.run(argv,init)}}); $.ui.invalidate('ui.render'); }
    return {};
  });
  on('prompt.submit', async ($, e, next) => {
    const host = {
      run: (request: unknown) => $.process.run([runtime.node, $.plugin.root + '/bridge/review.mjs'], {env: runtime.env, stdin: JSON.stringify(request), timeoutMs: (request as {command?: string}).command === 'scan' ? 600000 : 15000}),
      session: () => $.session.id(), now: () => $.clock.now(),
      enabled: () => $.store.get('personal_review_enabled'),
      save: (enabled: boolean) => $.store.set('personal_review_enabled', enabled),
      invalidate: () => { $.ui.invalidate('ui.render'); },
    };
    // The host API calls input from the prompt box 'composer'.
    if (e.origin.kind !== 'composer' || e.text.trimStart().startsWith('/')) return next(e);
    const current_generation = generation;
    const submission = ++submitted;
    let trace_id: string | undefined;
    let processing_started = false;
    const disposition = async (value: string) => {
      if (!trace_id) return;
      try {
        const saved = await $.process.run([runtime.node, $.plugin.root + '/bridge/cli.mjs', 'traces', 'disposition', trace_id, value], {env:runtime.env,timeoutMs:2000});
        if (saved.exitCode !== 0) trace_warning = 'recording gap: submission outcome was not saved';
      } catch { trace_warning = 'recording gap: submission outcome was not saved'; }
      if(trace_warning)$.ui.invalidate('ui.render');
    };
    const drop = async (reason: string) => {
      if (!trace_id && runtime.ready && processing_started) {
        try {
          const state=next.signal.aborted || generation !== current_generation ? 'cancelled':'blocked';
          const saved=await $.process.run([runtime.node,$.plugin.root+'/bridge/cli.mjs','traces','failure','adapter_failed',state],{env:runtime.env,timeoutMs:2000});
          if(saved.exitCode!==0)trace_warning='recording gap: failed submission was not saved';
        } catch {trace_warning='recording gap: failed submission was not saved';}
      }
      await disposition(next.signal.aborted || generation !== current_generation ? 'cancelled' : 'blocked');
      // A refusal stops delivery but does not restore the draft to the prompt box.
      // Never replace a newer draft or revive a cancelled submission.
      if (!next.signal.aborted && generation === current_generation && submission === submitted) {
        try {
          const draft = await $.prompt.read();
          if (!draft.text && !next.signal.aborted && generation === current_generation && submission === submitted) {
            await $.prompt.fill({text:e.text,mode:'replace'});
          }
        } catch {}
      }
      return {drop:reason};
    };
    if (!runtime.ready) return drop(runtime.status);
    try {
      if (new TextEncoder().encode(e.text).length > max_text_bytes) return drop('input exceeds the local processing limit; edit and resubmit');
      processing_started = true;
      const result = await $.process.run([runtime.node, $.plugin.root + '/bridge/cli.mjs', 'rewrite'], {
        env: runtime.env, stdin: JSON.stringify({version:3,text:e.text,assess:review.consent.live}), timeoutMs: 45000,
      });
      if (result.exitCode !== 0) return drop('local replacement failed; check savior doctor and resubmit');
      const answer = read_result(JSON.parse(result.stdout));
      trace_id = answer.trace_id;
      trace_warning = answer.trace_warning ? 'recording gap: this replacement was not saved' : '';
      if (next.signal.aborted || generation !== current_generation) return drop('submission cancelled; resubmit to retry');
      if(answer.assessment) await review.observe(host,e.text,answer.status==='error'?null:answer.text,answer.assessment);
      if (next.signal.aborted || generation !== current_generation) return drop('submission cancelled; resubmit to retry');
      if (answer.status === 'error') return drop('local replacement failed; check savior doctor and resubmit');
      const output_limit = Math.min(max_output_bytes, new TextEncoder().encode(e.text).length * 80 * 4);
      if (new TextEncoder().encode(answer.text).length > output_limit) return drop('invalid local replacement; resubmit to retry');
      if (submission === submitted) latest = { original: e.text, replacement: answer.text };
      $.ui.invalidate('ui.render');
      try {
        const forwarded = await next({ ...e, text: answer.text });
        await disposition('forwarded');
        return forwarded;
      } catch (error) { await disposition('forwarding_unknown'); throw error; }
    } catch {
      return drop('local replacement failed; check the draft and retry');
    }
  }).catch(async ($, e, next) => {
    if (next.called) return next(e);
    return { drop: 'local replacement failed; check the draft and retry' };
  });
  on('ui.render', { component: 'Pane' }, async ($, e, next) => {
    const host = {
      run: (request: unknown) => $.process.run([runtime.node, $.plugin.root + '/bridge/review.mjs'], {env: runtime.env, stdin: JSON.stringify(request), timeoutMs: (request as {command?: string}).command === 'scan' ? 600000 : 15000}),
      session: () => $.session.id(), now: () => $.clock.now(),
      enabled: () => $.store.get('personal_review_enabled'),
      save: (enabled: boolean) => $.store.set('personal_review_enabled', enabled),
      invalidate: () => { $.ui.invalidate('ui.render'); },
    };
    if (e.requestId !== 'savior' || e.surface !== 'terminal') return next(e);
    const { Box, Text, Button } = $.ui.resolve(e);
    if(review.view==='metrics')return <Box flexDirection="column">
      <Text bold>personal metrics — active profile only; no prompt text is saved</Text>
      <Button key="inspection" label="inspection" onPress={()=>{review.view='inspection';$.ui.invalidate('ui.render');}} />
      <Text>30-day metrics retention; history scores describe stored text, not recovered originals</Text>
      <Button key="live" hotkey="l" label={review.consent.live?'disable live metrics':'enable live metrics and sampled local judge'} onPress={()=>review.toggle(host,'live')} />
      <Button key="history" label={review.consent.history?'disable history access':'enable history access across this profile'} onPress={()=>review.toggle(host,'history')} />
      <Text>lookback: {review.days} days; {new Date(review.now-review.days*86400000).toLocaleDateString()} to {new Date(review.now).toLocaleDateString()}</Text>
      <Box>{[1,2,3,4,5].map(days=><Button key={`days-${days}`} label={String(days)} onPress={()=>{review.days=days;$.ui.invalidate('ui.render');}} />)}</Box>
      <Button key="scan" label={review.scanning?'scan running':'scan selected dates'} onPress={()=>review.scan(host)} />
      <Button key="cancel-review" label="cancel review" onPress={()=>review.cancel(host)} />
      <Button key="refresh" label="refresh metrics" onPress={async()=>{try{await review.refresh(host);}catch{review.message='metrics unavailable';}$.ui.invalidate('ui.render');}} />
      <Button key="export" label="export text-free metrics" onPress={()=>review.export(host)} />
      <Button key="delete" label="delete personal metrics" onPress={()=>review.clear(host)} />
      <Text>{review.message}</Text><Text>{review.summary('live')}</Text><Text>{review.categories('live')}</Text><Text>{review.summary('history')}</Text><Text>{review.categories('history')}</Text>
    </Box>;
    if(review.view==='policy')return <Box flexDirection="column">
      <Text bold>measured policy alignment — inconclusive</Text>
      <Text>model-directed abuse clause effective 2026-11-12; ordinary frustration, criticism and research are not automatically violations</Text>
      <Text>no published removal threshold; local judge estimates do not guarantee enforcement outcomes</Text>
      <Button key="inspection" label="inspection" onPress={()=>{review.view='inspection';$.ui.invalidate('ui.render');}} />
    </Box>;
    return <Box flexDirection="column">
      <Text bold>local inspection — never added to model context</Text>
      <Text>{runtime.status}</Text>
      <Button key="personal-metrics" hotkey="m" label="personal metrics" onPress={async()=>{review.view='metrics';review.now=await $.clock.now();$.ui.invalidate('ui.render');}} />
      <Button key="policy-evidence" label="policy evidence" onPress={()=>{review.view='policy';$.ui.invalidate('ui.render');}} />
      <Text>{latest ? '- ' + latest.original : 'no replacement in this session'}</Text>
      <Text>{trace_warning}</Text>
      <Text>{latest ? '+ ' + latest.replacement : ''}</Text>
    </Box>;
  });
}
