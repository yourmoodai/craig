const marker = '<!-- craig-adversarial-review -->';

function verdict(report) {
  if (typeof report.body !== 'string' || !report.body.trim() || report.body.length > 60000 || /co-authored-by|generated-by|assisted-by|reviewed with|🤖/i.test(report.body)) throw Error('invalid review body');
  const words = typeof report.overview === 'string' ? report.overview.trim().split(/\s+/).length : 0;
  const clean = words >= 50 && words <= 100 && report.qualified === true && report.complete === true && report.synthesis_complete === true && report.disposition === 'APPROVE' && Array.isArray(report.findings) && report.findings.length === 0 && Array.isArray(report.review_failures) && report.review_failures.length === 0;
  if (!report.body.includes('**Verdict: '+(clean ? 'Approved' : 'Request changes')+'**')) throw Error('review body disagrees with computed verdict');
  return clean;
}

module.exports = async function publish({github, context, core, head, pr, phase, report, evidence}) {
  const args = {...context.repo, pull_number:Number(pr)};
  async function current() {
    const pull = (await github.rest.pulls.get(args)).data;
    return pull.state === 'open' && !pull.draft && pull.head.sha === head;
  }
  if (!await current() || (report && report.commit_id !== head)) {core.notice('stale review discarded'); return;}
  let clean = false;
  let policy;
  const runUrl = `${context.serverUrl || 'https://github.com'}/${context.repo.owner}/${context.repo.repo}/actions/runs/${context.runId}`;
  let content;
  if (phase === 'waiting') content = 'Awaiting required tests. Model review has not started.';
  else if (phase === 'failed') content = 'Review unavailable: required tests or review execution failed. See the workflow for failing checks.\n\n**Verdict: Request changes**';
  else if (phase === 'complete') {clean = verdict(report); content = report.body;}
  else if (phase === 'policy') {
    const large = require('./large_pr.cjs');
    if (!large.testsPassed(evidence, head)) throw Error('size-policy approval requires passing tests');
    policy = await large.eligible({github,context,head,pr});
    if (!policy) throw Error('PR is not eligible for size-policy approval');
    clean = true;
    content = `This pull request contains ${policy.lines} added and deleted lines, exceeding the 5,000-line automatic-review threshold. Its author, @${policy.author}, qualifies through the ${policy.source}. Required verification, commit-message, and release-qualification workflows passed for this commit. The repository's large-PR policy therefore permits approval without model assessment. This approval records a policy exception; it does not claim that the code was inspected by a model or is free of defects.\n\n**Verdict: Approved**\n\nApproval basis: large PR authored by a repository code owner. Model review skipped.`;
  }
  else throw Error('invalid publication phase');
  const tests = evidence || report?.test_evidence;
  const links = tests?.head === head ? (tests.runs || []).filter(run=>Number.isSafeInteger(run.id) && run.id>0).map(run=>`[Test run ${run.id}](${context.serverUrl || 'https://github.com'}/${context.repo.owner}/${context.repo.repo}/actions/runs/${run.id})`).join(' · ') : '';
  const body = `${marker}\nCommit: \`${head}\` · [CI evidence and review run](${runUrl})\n${links}\n\n${content}`;
  const comments = await github.paginate(github.rest.issues.listComments, {...context.repo, issue_number:Number(pr), per_page:100});
  const existing = comments.filter(comment => comment.user?.login === 'github-actions[bot]' && comment.user?.type === 'Bot' && comment.body?.startsWith(marker)).sort((a,b)=>a.id-b.id)[0];
  // Recheck after pagination; a new commit may have arrived during the API calls.
  if (!await current()) {core.notice('stale review discarded'); return;}
  if (existing) {
    if (existing.body !== body) await github.rest.issues.updateComment({...context.repo, comment_id:existing.id, body});
  } else await github.rest.issues.createComment({...context.repo, issue_number:Number(pr), body});
  if (phase === 'policy') {
    const latest = await require('./large_pr.cjs').eligible({github,context,head,pr});
    if (!latest || latest.base !== policy.base) throw Error('size-policy target changed');
    const reviews = await github.paginate(github.rest.pulls.listReviews,{...args,per_page:100});
    if (!reviews.some(review=>review.user?.login==='github-actions[bot]' && review.user?.type==='Bot' && review.commit_id===head && review.state==='APPROVED')) {
      if (!await current()) throw Error('size-policy target changed');
      await github.rest.pulls.createReview({...args,commit_id:head,event:'APPROVE',body:'Approved under the repository policy for code-owner PRs over 5,000 changed lines after required tests passed. Model review was skipped; details are in the single review status comment.'});
    }
  }
  if (phase !== 'waiting' && !clean) core.setFailed('adversarial review did not establish a clean result');
};
module.exports.verdict = verdict;
