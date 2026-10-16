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
    await $.command.register({ name: 'craig', description: 'inspect the latest local replacement', immediate: true });
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
  on('command.run', { command: 'craig' }, async ($) => {
    await $.ui.open({ id: 'craig', title: 'local replacement', focus: true, closeOnEscape: true });
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
    if (e.origin.kind !== 'composer' || e.text.trimStart().startsWith('/')) return next(e);
    if (!runtime.ready) return { drop: runtime.status };
    const current_generation = generation;
    const submission = ++submitted;
    try {
      if (new TextEncoder().encode(e.text).length > max_text_bytes) return { drop: 'input exceeds the local processing limit; edit and resubmit' };
      const result = await $.process.run([runtime.node, $.plugin.root + '/bridge/cli.mjs', 'rewrite'], {
        env: runtime.env, stdin: JSON.stringify(review.consent.live ? {version:2,text:e.text,assess:true} : { version: 1, text: e.text }), timeoutMs: 45000,
      });
      if (next.signal.aborted || generation !== current_generation) return { drop: 'submission cancelled; resubmit to retry' };
      if (result.exitCode !== 0) return { drop: 'local replacement failed; check craig doctor and resubmit' };
      const answer = read_result(JSON.parse(result.stdout));
      if(answer.assessment) await review.observe(host,e.text,answer.status==='error'?null:answer.text,answer.assessment);
      if (next.signal.aborted || generation !== current_generation) return { drop: 'submission cancelled; resubmit to retry' };
      if (answer.status === 'error') return { drop: 'local replacement failed; check craig doctor and resubmit' };
      const output_limit = Math.min(max_output_bytes, new TextEncoder().encode(e.text).length * 80 * 4);
      if (new TextEncoder().encode(answer.text).length > output_limit) return { drop: 'invalid local replacement; resubmit to retry' };
      if (submission === submitted) latest = { original: e.text, replacement: answer.text };
      $.ui.invalidate('ui.render');
      return next({ ...e, text: answer.text });
    } catch {
      return { drop: 'local replacement failed; the draft was kept for retry' };
    }
  }).catch(async ($, e, next) => {
    if (next.called) return next(e);
    return { drop: 'local replacement failed; the draft was kept for retry' };
  });
  on('ui.render', { component: 'Pane' }, async ($, e, next) => {
    const host = {
      run: (request: unknown) => $.process.run([runtime.node, $.plugin.root + '/bridge/review.mjs'], {env: runtime.env, stdin: JSON.stringify(request), timeoutMs: (request as {command?: string}).command === 'scan' ? 600000 : 15000}),
      session: () => $.session.id(), now: () => $.clock.now(),
      enabled: () => $.store.get('personal_review_enabled'),
      save: (enabled: boolean) => $.store.set('personal_review_enabled', enabled),
      invalidate: () => { $.ui.invalidate('ui.render'); },
    };
    if (e.requestId !== 'craig' || e.surface !== 'terminal') return next(e);
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
      <Text>{latest ? '+ ' + latest.replacement : ''}</Text>
    </Box>;
  });
}
