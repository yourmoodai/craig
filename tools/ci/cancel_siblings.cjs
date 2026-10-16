const paths = new Set(['verify.yml', 'commit-messages.yml', 'release.yml', 'adversarial-review.yml'].map(name=>'.github/workflows/'+name));
const failures = new Set(['failure', 'timed_out', 'action_required', 'startup_failure']);
const terminal = new Set(['completed']);
const monitorName = 'cancel sibling ci on failure';

function eligible(run, head, repository) {
  return run.head_sha === head && paths.has(run.path) && run.head_repository?.full_name === repository &&
    (run.event === 'pull_request' || (['push','workflow_dispatch'].includes(run.event) && ['main','develop'].includes(run.head_branch)));
}

module.exports = async function monitor({github, context, head, core, sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms)), deadline=Date.now()+100*60*1000}) {
  const repository = `${context.repo.owner}/${context.repo.repo}`;
  while (Date.now() < deadline) {
    const runs = (await github.paginate(github.rest.actions.listWorkflowRunsForRepo, {...context.repo,head_sha:head,per_page:100}))
      .filter(run=>eligible(run,head,repository));
    let failed;
    let finished = runs.length > 0 && [...paths].every(path=>runs.some(run=>run.path===path));
    for (const run of runs) {
      if (failures.has(run.conclusion)) failed = run;
      if (!terminal.has(run.status)) {
        const jobs = await github.paginate(github.rest.actions.listJobsForWorkflowRun, {...context.repo,run_id:run.id,filter:'latest',per_page:100});
        const relevant = jobs.filter(job=>job.name!==monitorName);
        if (relevant.some(job=>failures.has(job.conclusion))) failed = run;
        // Our coordinator keeps its own workflow active after all other jobs finish.
        if (run.id !== context.runId || !relevant.length || relevant.some(job=>job.status!=='completed')) finished = false;
      }
    }
    if (failed) {
      core.notice(`ci failure in run ${failed.id}; cancelling siblings for ${head}`);
      const active = runs.filter(run=>run.status!=='completed').sort((a,b)=>(a.id===context.runId)-(b.id===context.runId));
      for (const run of active) {
        // Recheck immediately before cancellation; completed runs need no signal.
        const latest = (await github.rest.actions.getWorkflowRun({...context.repo,run_id:run.id})).data;
        if (!eligible(latest,head,repository) || latest.status==='completed') continue;
        try {await github.rest.actions.cancelWorkflowRun({...context.repo,run_id:run.id});}
        catch (error) {if (error.status!==409) throw error;}
      }
      return;
    }
    if (finished) return;
    await sleep(15000);
  }
  throw Error('ci cancellation monitor deadline exhausted');
};
module.exports.eligible = eligible;
