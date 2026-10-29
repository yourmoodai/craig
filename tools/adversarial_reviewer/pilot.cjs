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
      // Partial reruns copy completed jobs under new IDs and attempt numbers.
      // Only identical execution evidence can reuse the original reservation.
      const original = jobs.filter(candidate=>
        candidate.run_attempt <= job.run_attempt && candidate.name === job.name &&
        candidate.started_at && candidate.started_at === job.started_at &&
        candidate.completed_at && candidate.completed_at === job.completed_at &&
        JSON.stringify(candidate.steps) === JSON.stringify(job.steps))
        .reduce((attempt,candidate)=>Math.min(attempt,candidate.run_attempt),job.run_attempt);
      if (job.steps?.some(step=>step.name==='persist pilot reservation' && step.conclusion==='success') &&
          !reservations.some(a=>a.workflow_run?.id===run.id && a.name.endsWith('-'+original))) {
        throw Error('pilot reservation artifact is missing; refusing to reset spending');
      }
    }
  }
  const recoveries = [];
  const unresolved = new Set();
  for (const reserved of reservations) {
    const settled = finals.find(a=>a.name===reserved.name.replace('-reserved-','-settled-'));
    if (reserved.expired || !settled || settled.expired) {
      const key = reserved.name.slice((pilot+'-reserved-').length);
      if (!/^[0-9]+-[0-9]+$/.test(key)) throw Error('invalid reservation identity');
      const path = require('node:path').join(__dirname,'recoveries',key+'.json');
      if (!fs.existsSync(path)) throw Error(`pilot run ${reserved.workflow_run.id} reservation ${reserved.name} is unresolved or expired; remaining budget is unavailable`);
      const recovery = JSON.parse(fs.readFileSync(path,'utf8'));
      if (recovery.reservation_artifact_id !== reserved.id || recovery.run !== reserved.workflow_run.id ||
          `${recovery.run}-${recovery.attempt}` !== key || !Number.isSafeInteger(recovery.nanodollars) || recovery.nanodollars <= 0) throw Error('invalid recovery evidence');
      recoveries.push(recovery);
      unresolved.add(reserved.id);
    }
  }
  fs.mkdirSync('review-result',{recursive:true});
  fs.mkdirSync('review-result/previous-ledgers',{recursive:true});
  fs.writeFileSync('review-result/recoveries.json',JSON.stringify(recoveries));
  // Reconcile every branch of history; artifact IDs do not establish chronology.
  for (const reserved of reservations) {
    const settled = finals.find(a=>a.name===reserved.name.replace('-reserved-','-settled-'));
    if (unresolved.has(reserved.id)) continue;
    const archive = await github.rest.actions.downloadArtifact({...context.repo,artifact_id:settled.id,archive_format:'zip'});
    fs.writeFileSync(`review-result/previous-ledgers/${settled.id}.zip`,Buffer.from(archive.data));
  }
  fs.mkdirSync('review-result/previous-caches',{recursive:true});
  const caches = owned.filter(a=>a.name==='review-cache' && !a.expired &&
    Date.now()-Date.parse(a.created_at) <= 7*86400000).sort((a,b)=>Date.parse(b.created_at)-Date.parse(a.created_at)).slice(0,10);
  for (const artifact of caches) {
    if (artifact.size_in_bytes > 20000000) continue;
    const archive = await github.rest.actions.downloadArtifact({...context.repo,artifact_id:artifact.id,archive_format:'zip'});
    fs.writeFileSync(`review-result/previous-caches/${artifact.id}.zip`,Buffer.from(archive.data));
  }
  fs.writeFileSync('review-result/reservation.json',JSON.stringify({pilot,run:context.runId,attempt:process.env.GITHUB_RUN_ATTEMPT,maximum_usd:10}));
};
