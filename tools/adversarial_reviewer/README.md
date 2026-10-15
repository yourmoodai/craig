# Adversarial code review

Adapted from [awill1988/whip-it](https://github.com/awill1988/whip-it/tree/main/tools/adversarial_reviewer)
under the retained MIT license. Bedrock evaluates concrete failure scenarios against
numbered diff evidence; this is separate from Markdown and commit-message linting.

Run `python3 -m unittest discover -s tools/adversarial_reviewer` for transport,
evidence, coverage, and disposition tests. CI uses AWS CLI v2 and Bedrock Converse
with `us.anthropic.claude-haiku-4-5-20251001-v1:0` in `us-east-1`.
The application detector and personal-review judges remain offline.

GitHub Actions assumes `arn:aws:iam::463509296941:role/craig-bedrock-review`
through OIDC. The `bedrock-review` GitHub environment admits only the `main` and
`develop` branches. The role trusts only this repository/environment and the
`sts.amazonaws.com` audience. Its inline policy permits only `bedrock:InvokeModel`
for the selected US inference profile and its three destination models, with a
profile condition on foundation-model access. The exact policies are in
[`iam/trust.json`](iam/trust.json) and [`iam/invoke.json`](iam/invoke.json).
No AWS access keys or SSO credentials are stored in GitHub.

The environment variables `BEDROCK_REVIEW_ROLE_ARN` and `BEDROCK_REVIEW_MODEL`
select that role and model. Set `BEDROCK_REVIEW_ENABLED=true` only after account
model access and live qualification succeed. Bedrock is metered API usage, not
free Copilot credits. Request/output limits, deadlines, no SDK retries, and the
large-PR exception bound work but do not constitute a hard dollar spending cap.

For local qualification after SSO login:

```sh
AWS_PROFILE=awill AWS_REGION=us-east-1 \
BEDROCK_REVIEW_ENABLED=true \
BEDROCK_REVIEW_MODEL=us.anthropic.claude-haiku-4-5-20251001-v1:0 \
python3 tools/adversarial_reviewer/evaluate_model.py
```

Calls have no tools or agent execution. Only trusted checkout code runs; PR diffs
are data. A private temporary request file is removed after each invocation.
Responses must finish with `end_turn` and parse as a JSON object; token exhaustion,
filtered output, invalid JSON, and provider failures cannot approve. Whole JSON
Markdown fences are accepted because Haiku sometimes emits them. Transport errors
omit provider response bodies and review evidence. Only the publisher updates
the PR comment.

Qualification includes paired Node evaluation cases whose expected behavior is
checked by executing fixed fixtures. The reviewer receives observations from
fixed, reviewer-owned runtime probes to distinguish valid lexical shadowing from
duplicate declarations. These probes never execute code supplied by a PR.

Each review begins with a 50–100 word overview synthesized from all chunk results,
followed by **Approved** or **Request changes**. Findings and review failures appear
separately; chunk details and exclusions are collapsed below them.

One bot-owned PR comment tracks waiting for tests, failed prerequisites, and the
latest assessment. The publisher updates that comment rather than submitting
formal GitHub reviews for model assessments. The `adversarial verdict` CI job passes for a qualified,
complete, clean current-head assessment with a valid overview. Findings, unavailable
synthesis, and incomplete coverage fail the job. Existing branch protection still
requires a human approval; historical formal reviews remain unchanged.
Independent chunks continue after a failure within the shared time budget.

PRs with more than 5,000 added plus deleted lines can instead receive a size-policy
approval after all required tests pass. The author must have write access and be
listed in the base commit's repository-wide `.github/CODEOWNERS` rule. Path-specific
or team ownership uses normal review. PR #1 has an explicit exception for `awill1988`
against the initial base commit, which lacks CODEOWNERS. The policy updates the same
status comment, clearly states that model review was skipped, and submits one formal
approval per head commit. It does not change branch protection or merge the PR.

Non-executable PNG/JPEG/WebP files under `docs/assets` are excluded from code-review
coverage and listed as unreviewed assets. Other binary changes remain unresolved
coverage. Stale results are discarded. Model agreement is not proof of correctness.

Native verification and release jobs share dependency caches by runner image and
architecture; Core ML uses a separate namespace. Compiler, Cargo inputs, and Rust
flags remain part of the cache key. Builds and tests always execute. Job summaries
record cache hits, restore/build duration, and existing cache bytes for comparison;
cache size is sampled before the current job saves its cache.
