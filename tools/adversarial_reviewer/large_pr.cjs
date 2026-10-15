const initialBase = 'f0c8b668283eef344997e45740fb82774b949e2c';

function ownsRepository(text, author) {
  // Only the repository-wide ownership form is supported. Path rules or teams
  // require the normal review path rather than an incomplete ownership guess.
  const rules = text.split(/\r?\n/).map(line=>line.split('#')[0].trim()).filter(Boolean);
  if (rules.length !== 1) return false;
  const [pattern, ...owners] = rules[0].split(/\s+/);
  return pattern === '*' && owners.some(owner=>owner.toLowerCase() === '@'+author.toLowerCase());
}

async function eligible({github, context, head, pr}) {
  const pull = (await github.rest.pulls.get({...context.repo, pull_number:Number(pr)})).data;
  if (pull.state !== 'open' || pull.draft || pull.head.sha !== head || pull.user?.type !== 'User') return null;
  if (![pull.additions,pull.deletions].every(n=>Number.isSafeInteger(n) && n>=0)) return null;
  const lines = pull.additions + pull.deletions;
  if (lines <= 5000) return null;
  const access = (await github.rest.repos.getCollaboratorPermissionLevel({...context.repo,username:pull.user.login})).data.permission;
  if (!['admin','write','maintain'].includes(access)) return null;
  let source;
  try {
    const file = (await github.rest.repos.getContent({...context.repo,path:'.github/CODEOWNERS',ref:pull.base.sha})).data;
    if (file.encoding !== 'base64' || typeof file.content !== 'string') return null;
    if (!ownsRepository(Buffer.from(file.content,'base64').toString('utf8'),pull.user.login)) return null;
    source = 'base-commit CODEOWNERS';
  } catch (error) {
    if (error.status !== 404) throw error;
    // Explicit initial-import authorization; never trust ownership added by a PR.
    if (context.repo.owner !== 'yourmoodai' || context.repo.repo !== 'craig' || Number(pr)!==1 || pull.base.sha !== initialBase || pull.user.login !== 'awill1988') return null;
    source = 'initial-import owner exception';
  }
  return {lines, author:pull.user.login, base:pull.base.sha, source};
}

function testsPassed(evidence, head) {
  return evidence?.head === head && ['verify.yml','commit-messages.yml','release.yml'].every(file=>
    evidence.runs?.some(run=>run.workflow === '.github/workflows/'+file && run.conclusion === 'success'));
}
module.exports = {eligible, ownsRepository, testsPassed};
