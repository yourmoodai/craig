module.exports = async ({github, context, core}) => {
  const jobs = await github.paginate(github.rest.actions.listJobsForWorkflowRunAttempt, {...context.repo, run_id:context.runId, attempt_number:Number(process.env.GITHUB_RUN_ATTEMPT), per_page:100});
  const job = jobs.find(job => job.name === process.env.CACHE_JOB);
  const seconds = step => step?.started_at && step?.completed_at ? String((Date.parse(step.completed_at)-Date.parse(step.started_at))/1000) : 'unavailable';
  const caches = await github.paginate(github.rest.actions.getActionsCacheList, {...context.repo, ref:process.env.GITHUB_REF, per_page:100});
  const bytes = caches.filter(cache => cache.key.includes(process.env.CACHE_GROUP)).reduce((sum,cache)=>sum+cache.size_in_bytes,0);
  await core.summary.addHeading('Native cache measurements', 3).addTable([
    ['Measurement','Value'],
    ['Namespace',process.env.CACHE_GROUP],
    ['Exact cache hit',process.env.CACHE_HIT || 'false'],
    ['Restore seconds',seconds(job?.steps.find(step=>step.name==='restore rust dependencies'))],
    ['Build seconds',seconds(job?.steps.find(step=>step.name==='native build'))],
    ['Existing cache bytes for this ref and namespace',String(bytes)],
  ]).addRaw('\nCache size is measured before this job saves its cache. Compare cold and warm runs on the same runner and toolchain.\n').write();
};
