# Adversarial code review

Adapted from [awill1988/whip-it](https://github.com/awill1988/whip-it/tree/main/tools/adversarial_reviewer)
under the retained MIT license. Kimi evaluates concrete failure scenarios against
numbered diff evidence; this is separate from Markdown and commit-message linting.

Run `python3 -m unittest discover -s tools/adversarial_reviewer` for transport,
evidence, coverage, and disposition tests. With `KIMI_API_KEY` configured, run
`python3 tools/adversarial_reviewer/evaluate_model.py` for live qualification.

Qualification includes paired Node evaluation cases whose expected behavior is
checked by executing fixed fixtures. The reviewer receives observations from
fixed, reviewer-owned runtime probes to distinguish valid lexical shadowing from
duplicate declarations. These probes never execute code supplied by a PR.

Each review begins with a 50–100 word overview synthesized from all chunk results,
followed by **Approved** or **Request changes**. Findings and review failures appear
separately; chunk details and exclusions are collapsed below them.

One bot-owned PR comment tracks waiting for tests, failed prerequisites, and the
latest assessment. The publisher updates that comment rather than submitting
formal GitHub reviews. The `adversarial verdict` CI job passes only for a qualified,
complete, clean current-head assessment with a valid overview. Findings, unavailable
synthesis, and incomplete coverage fail the job. Existing branch protection still
requires a human approval; historical formal reviews remain unchanged.
Independent chunks continue after a failure within the shared time budget.

Non-executable PNG/JPEG/WebP files under `docs/assets` are excluded from code-review
coverage and listed as unreviewed assets. Other binary changes remain unresolved
coverage. Stale results are discarded. Model agreement is not proof of correctness.

Native verification and release jobs share dependency caches by runner image and
architecture; Core ML uses a separate namespace. Compiler, Cargo inputs, and Rust
flags remain part of the cache key. Builds and tests always execute. Job summaries
record cache hits, restore/build duration, and existing cache bytes for comparison;
cache size is sampled before the current job saves its cache.
