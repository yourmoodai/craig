const required = ['verify.yml', 'commit-messages.yml', 'release.yml'];

function select_runs(runs, head) {
  return required.map(file => {
    const matching = runs.filter(run => run.head_sha === head && run.event === 'pull_request' && run.path === '.github/workflows/' + file);
    return matching.sort((a, b) => b.id - a.id)[0];
  });
}

module.exports = async function gate({github, context, head, pr, output, sleep = ms => new Promise(resolve => setTimeout(resolve, ms))}) {
  const deadline = Date.now() + 60 * 60 * 1000;
  while (Date.now() < deadline) {
    const pull = (await github.rest.pulls.get({...context.repo, pull_number:Number(pr)})).data;
    if (pull.state !== 'open' || pull.draft || pull.head.sha !== head) throw Error('review target is stale');
    const runs = await github.paginate(github.rest.actions.listWorkflowRunsForRepo, {...context.repo, head_sha:head, event:'pull_request', per_page:100});
    const selected = select_runs(runs, head);
    if (selected.some(run => run?.status === 'completed' && run.conclusion !== 'success')) throw Error('required tests did not pass');
    if (selected.every(run => run?.status === 'completed' && run.conclusion === 'success')) {
      const evidence = {head, runs:[]};
      for (const run of selected) {
        const jobs = await github.paginate(github.rest.actions.listJobsForWorkflowRunAttempt, {...context.repo, run_id:run.id, attempt_number:run.run_attempt, per_page:100});
        evidence.runs.push({workflow:run.path, id:run.id, attempt:run.run_attempt, url:run.html_url, conclusion:run.conclusion,
          jobs:jobs.map(job => ({name:job.name, conclusion:job.conclusion, steps:job.steps.map(step => ({name:step.name, conclusion:step.conclusion}))}))});
      }
      require('node:fs').writeFileSync(output, JSON.stringify(evidence));
      return;
    }
    await sleep(30000);
  }
  throw Error('required tests did not finish before review deadline');
};
module.exports.select_runs = select_runs;
