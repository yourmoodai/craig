# Adversarial code review

Adapted from [awill1988/whip-it](https://github.com/awill1988/whip-it/tree/main/tools/adversarial_reviewer)
under the retained MIT license. Bedrock evaluates concrete failure scenarios against
numbered diff evidence; this is separate from Markdown and commit-message linting.

Run `python3 -m unittest discover -s tools/adversarial_reviewer` for transport,
evidence, coverage, and disposition tests. CI uses AWS CLI v2 and Bedrock Converse
with `deepseek.v3.2` in `us-east-1`.
The application detector and personal-review judges remain offline.

GitHub Actions assumes `arn:aws:iam::463509296941:role/savior-bedrock-review`
through OIDC. The `bedrock-review` GitHub environment admits only the `main` and
`develop` branches. The role trusts only this repository/environment and the
`sts.amazonaws.com` audience. Its inline policy permits only `bedrock:InvokeModel`
for DeepSeek V3.2 in `us-east-1`. The exact policies are in
[`iam/trust.json`](iam/trust.json) and [`iam/invoke.json`](iam/invoke.json).
No AWS access keys or SSO credentials are stored in GitHub.

The environment variables `BEDROCK_REVIEW_ROLE_ARN` and `BEDROCK_REVIEW_MODEL`
select that role and model. Set `BEDROCK_REVIEW_ENABLED=true` after account model access is verified.
Every enabled run requires passing qualification for the exact reviewer configuration; successful qualification is reusable for 24 hours. Bedrock is metered API usage, not
free Copilot credits. The pilot shares a $10 inference budget, including at most $2 for qualification
and strategy comparison. An atomic SQLite ledger reserves 167,936 input tokens (164 Ki tokens) plus
maximum output before every call. Actual usage releases unused reservations;
missing usage and interrupted calls retain their maximum cost. Budget exhaustion
leaves coverage incomplete. This bounds modeled inference cost at the pinned US
rates ($0.62/M input, $1.85/M output), not taxes or unrelated AWS usage.

CI serializes the pilot across PRs. The original `haiku45-pilot-20261015`
artifact namespace is retained so changing models cannot reset spending history.
The ledger also retains $0.03 reserved for the earlier account-access probes. It uploads a reservation before inference and
reconciles it with a durable ledger afterward. Resumed runs restore that ledger;
an unresolved, deleted, or expired reservation prevents further spending. Keep
these artifacts for the pilot's lifetime. The ledger contains token metrics and
prompt hashes, not prompt text. Use CI as the pilot's sole spending authority;
local inference must not use an independent copy of the $2 allowance.

For a local packing dry run (no AWS calls or inference charges):

```sh
AWS_PROFILE=awill AWS_REGION=us-east-1 \
python3 tools/adversarial_reviewer/adversarial_review.py \
  --base origin/main --head HEAD --strategy adaptive --dry-run \
  --json-output .cache/review-pilot/adaptive-manifest.json
```

Use `--strategy baseline` for one primary file per request, bounded at 48 KiB. Manifests list primary units,
stable evidence IDs, full input estimates, repeated supporting context, omissions,
and maximum first-pass cost. Verification and subdivision need additional funds.
The workflow records both manifests, qualification results, usage, and latency.

A primary unit is a contiguous addition/deletion block plus its assigned context.
The adaptive primary-diff ceilings are 4,000 tokens for up to eight units, 8,000
for nine through 32, and 16,000 thereafter. Packing prefers the same file, changed
references, then directories; ordinary batches admit at most eight files and
16 units. Oversized units split on source lines, retaining old/new coordinates.
Renames and mode-only changes have file-level evidence. Deletions remain reviewable.
Consecutive evidence locations use compact ranges in prompts; stable IDs remain
in the manifest, avoiding a repeated file path and identifier on every source line.
Each primary unit has one owner; supporting context does not increase coverage.

DeepSeek V3.2 does not support Bedrock `CountTokens`. Packing estimates each UTF-8
byte as one token, plus 1,024 template tokens and 128 per message; this is not an
exact tokenizer count. Rendered prompts must fit 48,000 estimated tokens and
128 KiB. Actual input above 48,000 tokens prevents accepting the response.
Spending reservations use full context capacity independently of this estimate. Supplemental context is capped at 8,000 tokens
(currently an intentionally conservative 8,000 UTF-8 bytes); omitted support is
listed. Assessments permit 1,024 output tokens, verification 1,536, and overviews 256. An explicitly truncated assessment or verification gets one retry at 4,096 tokens. Truncated responses or three findings trigger subdivision, never approval
from a partial parent. Indivisible changes, missing context, or exhausted budget
remain incomplete. Provider prompt caching is disabled. Validated assessment results are reusable for seven days from trusted workflow artifacts. Cache keys include model settings, reviewer implementation, diff, supporting evidence, and test context. Incomplete results are never cached. Formatting-only changes reuse assessments; approval and current-head coverage are computed again.

Qualification exercises benign changes and seeded defects, then compares both
strategies on the same clean and four-defect batches. Adaptive packing is selected
only when coverage and expected labels survive and its measured cost is lower;
otherwise the qualified baseline remains selected. Full-PR plans must also fit
the 512-assessment limit; an oversized adaptive plan falls back to baseline. Neither strategy is qualified
by unit tests alone. Account model access remains a prerequisite for the live comparison.

Calls have no tools or agent execution. Only trusted checkout code runs; PR diffs
are data. A private temporary request file is removed after each invocation.
Responses must finish with `end_turn` and parse as a JSON object; token exhaustion,
filtered output, invalid JSON, and provider failures cannot approve. Whole JSON
Markdown fences are accepted because models sometimes emit them. Transport errors
omit provider response bodies and review evidence. Only the publisher updates
the PR comment.

Qualification includes paired Node evaluation cases whose expected behavior is
checked by executing fixed fixtures. The reviewer receives observations from
fixed, reviewer-owned runtime probes to distinguish valid lexical shadowing from
duplicate declarations. These probes never execute code supplied by a PR.

Each review begins with a 50–100 word overview synthesized from all chunk results,
followed by **Approved** or **Request changes**. Concise findings appear below the verdict; full evidence and exclusions remain in the artifact.

One bot-owned PR comment tracks review progress, failed prerequisites, and the
latest assessment. It includes a 50–100 word overview, verdict, coverage, and
committed cost. A qualified, complete, clean current-head assessment submits a
formal GitHub approval. Independent verification, commit-message, and release checks
must also pass before the PR is ready to merge. Validated findings submit `REQUEST_CHANGES`; infrastructure
failures fail the check without submitting a defect review. Repeated publication
updates the same comment and does not duplicate an unchanged formal verdict.
There is no large-PR or code-owner approval bypass. Branch protection is unchanged.

Non-executable PNG/JPEG/WebP files under `docs/assets` are excluded from code-review
coverage and listed as unreviewed assets. Other binary changes remain unresolved
coverage. Stale results are discarded. Model agreement is not proof of correctness.

Native verification and release jobs share dependency caches by runner image and
architecture; Core ML uses a separate namespace. Compiler, Cargo inputs, and Rust
flags remain part of the cache key. Builds and tests always execute. Job summaries
record cache hits, restore/build duration, and existing cache bytes for comparison;
cache size is sampled before the current job saves its cache.

Model limits and API support are documented in the [AWS DeepSeek V3.2 model card](https://docs.aws.amazon.com/bedrock/latest/userguide/model-card-deepseek-deepseek-v3-2.html).
Usage and stop reasons come from [Converse](https://docs.aws.amazon.com/bedrock/latest/APIReference/API_runtime_Converse.html).
Pilot rates use [Bedrock standard pricing](https://aws.amazon.com/bedrock/pricing/);
recheck rates before authorizing another pilot.

The single PR comment is limited to 2,500 characters: a 50–100-word overview, verdict, coverage, cost, and up to three concise findings. Full findings and chunk evidence remain in the `adversarial-report` artifact.
