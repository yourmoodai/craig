import test from 'node:test';
import assert from 'node:assert/strict';
import gate from '../tools/adversarial_reviewer/author_gate.cjs';

function setup(content, permission = 'write') {
  const refs = [];
  const github = {rest: {repos: {
    getContent: async ({ref}) => {
      refs.push(ref);
      if (content === undefined) throw Object.assign(Error('missing'), {status: 404});
      return {data: {type: 'file', encoding: 'base64', content: Buffer.from(content).toString('base64')}};
    },
    getCollaboratorPermissionLevel: async () => ({data: {permission}}),
  }}};
  return {github, context: {repo: {owner: 'org', repo: 'repo'}},
    pr: {base: {sha: 'trusted-base'}, head: {sha: 'untrusted-head'}, user: {login: 'owner'}}, refs};
}

test('only a named owner with write access is eligible, using the base revision', async () => {
  const args = setup('* @owner\n');
  assert.equal(await gate(args), true);
  assert.deepEqual(args.refs, ['trusted-base']);
  assert.equal(await gate(setup('* @other # @owner\n')), false);
  assert.equal(await gate(setup('* @owner\n', 'read')), false);
  assert.equal(await gate(setup('* @org/team\n')), false);
});

test('bootstrap is used only when all base CODEOWNERS locations are absent', async () => {
  assert.equal(await gate({...setup(undefined), bootstrap: '@owner'}), true);
  assert.equal(await gate({...setup('* @other'), bootstrap: '@owner'}), false);
  assert.equal(await gate(setup(undefined)), false);
});

test('API failures cannot enable bootstrap authorization', async () => {
  const args = setup(undefined);
  args.github.rest.repos.getContent = async () => {throw Object.assign(Error('forbidden'), {status: 403});};
  await assert.rejects(gate({...args, bootstrap: '@owner'}), /forbidden/);
});
