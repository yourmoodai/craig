const required = ['verify.yml', 'commit-messages.yml', 'release.yml'];

function select_runs(runs, head) {
  return required.map(file => {
    const matching = runs.filter(run => run.head_sha === head && run.event === 'pull_request' && run.path === '.github/workflows/' + file);
    return matching.sort((a, b) => b.id - a.id)[0];
  });
}

module.exports = async function snapshot({github, context, head, pr, output}) {
  const pull = (await github.rest.pulls.get({...context.repo, pull_number:Number(pr)})).data;
  if (pull.state !== 'open' || pull.draft || pull.head.sha !== head) throw Error('review target is stale');
  const runs = await github.paginate(github.rest.actions.listWorkflowRunsForRepo, {...context.repo, head_sha:head, event:'pull_request', per_page:100});
  const evidence = {head, observed_at:new Date().toISOString(), runs:[]};
  for (const [index, run] of select_runs(runs, head).entries()) {
    if (!run) {
      evidence.runs.push({workflow:'.github/workflows/'+required[index], status:'not_started', conclusion:null, jobs:[]});
      continue;
    }
    const jobs = await github.paginate(github.rest.actions.listJobsForWorkflowRunAttempt, {...context.repo, run_id:run.id, attempt_number:run.run_attempt, per_page:100});
    evidence.runs.push({workflow:run.path, id:run.id, attempt:run.run_attempt, status:run.status, conclusion:run.conclusion,
      jobs:jobs.map(job=>({name:job.name, status:job.status, conclusion:job.conclusion,
        steps:(job.steps || []).map(step=>({name:step.name, status:step.status, conclusion:step.conclusion}))}))});
  }
  require('node:fs').writeFileSync(output, JSON.stringify(evidence));
};
module.exports.select_runs = select_runs;
