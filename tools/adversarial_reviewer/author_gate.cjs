// Authorization comes from the base branch, never the proposed CODEOWNERS file.
module.exports = async function author_gate({github, context, pr, bootstrap = ''}) {
  let content;
  for (const path of ['.github/CODEOWNERS', 'CODEOWNERS', 'docs/CODEOWNERS']) {
    try {
      const {data} = await github.rest.repos.getContent({...context.repo, path, ref: pr.base.sha});
      if (data.type !== 'file' || data.encoding !== 'base64') throw Error('unsupported codeowners response');
      content = Buffer.from(data.content, 'base64').toString('utf8');
      break;
    } catch (error) {
      if (error.status !== 404) throw error;
    }
  }
  const owners = content === undefined ? bootstrap.split(/\s+/) : content.split('\n')
    .flatMap(line => line.split('#')[0].trim().split(/\s+/).slice(1));
  const author = pr.user.login.toLowerCase();
  if (!owners.some(owner => /^@[a-z0-9-]+$/i.test(owner) && owner.slice(1).toLowerCase() === author)) return false;
  const {data} = await github.rest.repos.getCollaboratorPermissionLevel({...context.repo, username: pr.user.login});
  return ['admin', 'maintain', 'write'].includes(data.permission);
};
