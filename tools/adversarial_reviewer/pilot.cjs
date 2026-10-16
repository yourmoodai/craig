// Artifact reservations survive runner loss. Missing reconciliation consumes the pilot.
const fs = require('node:fs');
const pilot = 'haiku45-pilot-20261015';

module.exports = async function restore({github,context}) {
  const artifacts = await github.paginate(github.rest.actions.listArtifactsForRepo,{...context.repo,per_page:100});
  const allRuns = await github.paginate(github.rest.actions.listWorkflowRuns,{...context.repo,workflow_id:'adversarial-review.yml',created:'>=2026-10-15',per_page:100});
  const runs = allRuns.filter(run=>run.event==='pull_request_target' ||
    (['push','workflow_dispatch'].includes(run.event) && ['main','develop'].includes(run.head_branch)));
  const owned = artifacts.filter(a=>runs.some(run=>run.id===a.workflow_run?.id));
  const reservations = owned.filter(a=>a.name.startsWith(pilot+'-reserved-'+a.workflow_run.id+'-'));
  const finals = owned.filter(a=>a.name.startsWith(pilot+'-settled-'+a.workflow_run.id+'-'));
  for (const run of runs) {
    const jobs = await github.paginate(github.rest.actions.listJobsForWorkflowRun,{...context.repo,run_id:run.id,filter:'all',per_page:100});
    for (const job of jobs) {
      if (job.steps?.some(step=>step.name==='persist pilot reservation' && step.conclusion==='success') &&
          !reservations.some(a=>a.workflow_run?.id===run.id && a.name.endsWith('-'+job.run_attempt))) {
        throw Error('pilot reservation artifact is missing; refusing to reset spending');
      }
    }
  }
  for (const reserved of reservations) {
    const settled = finals.find(a=>a.name===reserved.name.replace('-reserved-','-settled-'));
    if (reserved.expired || !settled || settled.expired) throw Error('pilot has an unresolved or expired reservation; remaining budget is unavailable');
  }
  fs.mkdirSync('review-result',{recursive:true});
  fs.mkdirSync('review-result/previous-ledgers',{recursive:true});
  // Reconcile every branch of history; artifact IDs do not establish chronology.
  for (const reserved of reservations) {
    const settled = finals.find(a=>a.name===reserved.name.replace('-reserved-','-settled-'));
    const archive = await github.rest.actions.downloadArtifact({...context.repo,artifact_id:settled.id,archive_format:'zip'});
    fs.writeFileSync(`review-result/previous-ledgers/${settled.id}.zip`,Buffer.from(archive.data));
  }
  fs.writeFileSync('review-result/reservation.json',JSON.stringify({pilot,run:context.runId,attempt:process.env.GITHUB_RUN_ATTEMPT,maximum_usd:10}));
};
