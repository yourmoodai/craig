# Adversarial code review

Adapted from [awill1988/whip-it](https://github.com/awill1988/whip-it/tree/main/tools/adversarial_reviewer)
under the retained MIT license. Copilot evaluates concrete failure scenarios against
numbered diff evidence; this is separate from Markdown and commit-message linting.

Run `python3 -m unittest discover -s tools/adversarial_reviewer` for transport,
evidence, coverage, and disposition tests. CI uses Copilot CLI 1.0.98 and the
organization-scoped `GITHUB_TOKEN`; no separate provider key is needed.

Live calls default to disabled. Before setting the repository variable
`COPILOT_BILLING_VERIFIED=true`, an organization owner must verify usable included
credits, a hard no-overage spending control, and the policy allowing Copilot CLI
to bill `yourmoodai`. This variable records that verification; it does not enforce
GitHub billing limits itself. Set `COPILOT_REVIEW_MODEL` to an explicitly allowed
model identifier, then run workflow qualification before relying on its verdict.
Organization access is not a promise of free usage. No paid-provider fallback is
configured. Missing entitlement or exhausted credits leaves review unavailable.

The CLI runs in an empty temporary directory with an isolated home, no tools,
no built-in MCP servers, and no custom repository instructions. Review evidence
is supplied in the prompt. Only the publisher updates the PR comment.

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
