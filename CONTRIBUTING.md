# Contributing to craig

Bug reports, design proposals, documentation corrections, tests, and code are
welcome. Start with an issue so contributors and maintainers agree on the
problem before implementing a change.

## Issue acceptance

Search [open and closed issues](https://github.com/yourmoodai/craig/issues?q=is%3Aissue)
before opening a proposal. Describe the problem, proposed behavior, a small
example, and effects on privacy, latency, compatibility, or model quality.

Community implementation starts when the maintainer applies `status: accepted`
or explicitly says the issue is ready for implementation. Acknowledgment,
questions, and ongoing design discussion are not acceptance. Issues labeled
`good first issue` or `help wanted` are accepted unless a maintainer says
otherwise. This also applies to small documentation changes.

Changes authored by [@awill1988](https://github.com/awill1988) are exempt from
issue acceptance because the maintainer supplies the prioritization directly.
The same technical and verification standards apply. An early community PR
may be closed and redirected to an issue; it can be reopened after acceptance.

## Maintainer commitments

The maintainer will acknowledge issues and initially review eligible PRs within
seven calendar days. Responses will state whether a proposal is accepted,
needs information or design discussion, duplicates existing work, or falls
outside the project's direction. Declined proposals receive a reason, and
blocked reviews receive a status update. Feedback stays specific and respectful.

Acknowledgment does not guarantee acceptance or merging. If seven days pass
without a response, one follow-up on the issue or PR is welcome.

## Design constraints

- Detection, source offsets, protection checks, and replacement belong in Rust.
  JavaScript and agent adapters transport requests and results.
- Runtime inference stays local and offline. Downloads require explicit setup.
  Never log or persist original prompts, or use private history as training data.
- Preserve unmatched text exactly. Protected or failed replacements retain the
  draft; never submit the original as a fallback.
- Keep input coverage intact when measuring speed. Report resident rewriting,
  process startup, host interaction, and optional judging separately.
- Separate replacement correctness, reference-label agreement, and model
  agreement. Keep provisional labels visible. Policy evaluation is measured
  policy alignment, not certification or provider approval.

Discuss model changes, public interfaces, supported agents, dependencies,
licensing, and release behavior in the accepted issue. The
[architecture decision](docs/adr/0001-local-span-replacement.md) owns these
boundaries. New implementation phases do not automatically need new ADRs.

## Development

Work on a focused branch in your fork. Source development requires Rust, a
C/C++ toolchain, and Node.js 22.18 or newer. Build the Rust library and typed
Node bindings before exercising the adapter.

```sh
npm ci
npm run build
npm run model:fetch
export CRAIG_MODEL_DIR="$PWD/.cache/models/mudes-en-base"
node bridge/cli.mjs doctor
claude --plugin-dir "$PWD"
npm run verify
```

Model changes additionally require `npm run eval:local` and the reference-parity
checks in [evaluation](docs/evaluation.md). That guide also covers debug and
release builds, memory profiling, quality review, and the 500 ms qualification
protocol. Shared CI hardware does not qualify latency. An installed-host test
is separate from adapter unit tests; state explicitly when it cannot run.

Add focused tests for changed behavior. Update documentation for changed
interfaces and contributor workflows. Test and profiling data must be synthetic
or explicitly licensed for that purpose.

## Commit messages and hooks

Use lowercase Conventional Commit subjects, optionally preceded by a ticket
such as `[PROJ-123]`. Subjects must fit within 50 characters and have no trailing
period; wrap bodies at 72 characters. Do not add generated attribution,
co-author footers, or tool signatures.

The Rust [commit checker](tools/commit_check/README.md) runs in CI and is
available through `.githooks/commit-msg`. Before configuring hooks, inspect the
existing path:

```sh
git config --get core.hooksPath
```

If no path is configured, activate the repository hooks with
`git config core.hooksPath .githooks`. If a developer profile already manages
hooks, preserve its signing, routing, and push checks. Use the checker directly
or integrate it through that profile's supported hook mechanism; do not replace
the configured path merely to install these hooks.

Existing published bootstrap commits predate commit-format enforcement. New
commits and PR titles are checked without rewriting that history.

## Pull requests

Link the accepted issue when required. Describe the resulting behavior,
affected modules and adapters, verification, limitations, and manual steps.
Use a conventional lowercase PR title. Keep review updates in new commits;
do not rewrite shared history. Green CI is required but does not replace
maintainer review or the model-promotion gates.

## Licensing

Submit only material you have the right to contribute. Contributions use the
project's existing [LICENSE](LICENSE) and [NOTICE](NOTICE) terms. Model weights
and imported tooling retain their separate licenses; keep their provenance and
notices intact. Synthetic fixtures use `CC0-1.0` where declared.


## Release and review workflows

The [architecture decision](docs/adr/0001-local-span-replacement.md#release-distribution-and-initialization)
owns release packaging and activation. Run `python3 scripts/release/package.py stage`
with a supported target and output directory after a release build. CI assembles
all targets, exercises the archive, and gates tag publication on installed-host tests.
The generated release marketplace points at a checksummed ZIP; never point it at
an unqualified candidate. Source archives are not installable release bundles.

Kimi waits for required tests, then updates one PR comment with a 50–100 word
overview and an Approved or Request changes verdict. The `adversarial verdict`
CI job passes only for qualified, complete, clean reviews with valid synthesis;
findings or review failures fail the job. It does not submit formal GitHub reviews.
Static documentation images are explicitly excluded from code-review coverage.
See [reviewer verification](tools/adversarial_reviewer/README.md). Existing required
human/PR approvals and stale-review dismissal remain in effect. The environment
secret is `KIMI_API_KEY` in `default`; never put its value in code or logs.
