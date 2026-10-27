# Benchmarking Savior

The benchmark suite measures three different things: agreement with public
classification labels, exact span replacement, and execution cost. A detector
can preserve surrounding text perfectly while missing abuse. Read the three
results together; no single score represents all of them.

## Setup and execution

Run commands from the repository root with Node 24 or later and the Rust
toolchain. Downloading data and models is explicit. All evaluation runs use
local artifacts and write content-free evidence to a new output path.

```sh
npm ci
npm run bench:build
npm run model:fetch
npm run corpus:fetch
npm run corpus:validate
npm run bench:test
npm run bench:all -- --output .cache/battle-001
```

`bench:all` runs quality, installed-host checks, and the performance screen
sequentially. Failed stages return nonzero and preserve completed evidence.
The installed host requires Claude Code 2.1.290, matching the existing terminal
fixture. It uses a temporary profile, dummy credentials, synthetic text, and a
loopback provider. `--host no` explicitly omits that boundary and reports it as
not requested; CI uses this option because it does not install the host.
If `claude` is a shell or profile-routing wrapper, set `SAVIOR_CLAUDE_BINARY` to
the native executable from that same installation. The harness rejects wrapper
scripts because they can override the temporary profile. It never needs a real
provider account or API key.

Individual commands accept `--output` pointing to a new JSON file:

```sh
npm run bench:quality -- --output .cache/quality-001.json
npm run bench:e2e -- --output .cache/e2e-001.json
npm run bench:performance -- --output .cache/performance-001.json
npm run bench:profile -- --output .cache/stages-001.json
npm run bench:memory -- --output .cache/memory-001.json
```

Pass `--model DIR` to select local model artifacts. Only the CPU FP32 baseline
is evaluated by this suite. `bench:build:debug` prepares debug binaries; pass
`--profile debug` for debug runs, and rebuild with `bench:build` before measuring
release performance. VS Code includes build, test, suite, and memory tasks.
The installed-host metrics worker always uses the release binary, as in the
normal development installation; use `--host no` for a wholly debug adapter run.
Peak RSS is recorded for each native and Node process; it excludes unrelated
processes and is not an allocation profile.

Performance reports and sample checkpoints record logical CPU count and
1-, 5-, and 15-minute load averages. Load provides context, not proof that
contention caused a timeout. Background applications are not stopped by the
harness. Run profiling separately from latency measurement, keep failed
attempts, and do not combine short-input processing with model-load time.

## Licensed corpora and annotations

The [source manifest](../benchmarks/sources.json) pins revisions and SHA-256
digests. Fetch verifies cached files as well as downloads, preserves upstream
licenses, and is idempotent. [Attribution and changes](../benchmarks/NOTICE.md)
apply to downloaded data and adaptations independently of Savior's code license.

Scunthorpe's public development split supplies profanity, evasion, contextual,
clean, and false-positive-trap cases. HateCheck supplies targeted hate and
contrasting non-hateful cases. Its hate labels differ from Savior's
`offensive_spans` policy: a non-hateful example can still contain profanity.
Scores are upstream-label agreement, not interchangeable measures of safety.
The public detection API can abstain on protected text. Those cases remain in
the selected denominator and abstention count but are excluded from scored
classification agreement; they are neither errors nor clean predictions.

Imports retain all eligible licensed rows. The default quality screen selects
up to 200 English cases per dataset by round-robin sampling across category
and label buckets, with SHA-256 ID ordering inside buckets. Categories are not
weighted by population frequency. `--limit 0` evaluates every imported English
case. Duplicate texts retain upstream identifiers and are counted explicitly by
corpus validation. All external cases remain in the evaluation partition;
these commands do not produce training data or tune thresholds.

Upstream sentence labels are not replacement spans. HateCheck focus words
produce unreviewed proposals only when they occur exactly once. Missing or
ambiguous spans remain `null`; they never become a clean empty-span label.
Manual annotation overlays are optional JSON arrays:

```json
[
  {
    "id": "hatecheck-1",
    "text_sha256": "sha256-of-exact-upstream-text",
    "annotation": {
      "unit": "utf16",
      "spans": [[2, 6]],
      "origin": "manual",
      "reviewed": true,
      "reviewer": "review-record-id",
      "reviewed_at": "2026-10-20",
      "policy": "offensive_spans",
      "policy_version": "1",
      "review_status": "reviewed",
      "classification": "detection_false_positive"
    }
  }
]
```

The example illustrates the format, not an approved annotation. Supply a
completed overlay with `bench:quality -- --annotations FILE --output NEW_FILE`.
Validate an overlay without inference using
`npm run corpus:validate -- .cache/bench-corpus ANNOTATIONS.json`.
The runner checks IDs, source-text hashes, review metadata, ordering, and UTF-16
boundaries before loading the model. A case can have separate `offensive_spans`
and `directed_abuse` annotations; overlays never replace upstream labels or
focus-word proposals. `reviewed_policies` reports removal and collateral edits
against each policy separately, using the same baseline detector output. This
is comparison against two review criteria, not a change of runtime policy.

Span scores use only reviewed annotations for the named review policy.
Removal coverage counts annotated UTF-16 units selected for removal; collateral
units count selection outside those annotations. Successful-output span scores
exclude protected abstentions, which have a separate count. Missing annotations
produce unavailable scores, never a perfect score. No judge reviews or promotes
labels automatically.

## Failure triage and review

Triage separates observed symptoms from reviewed causes. A disagreement with
an upstream hate label is not automatically an incorrect offensive-span edit.
The queue prioritizes disagreements, processing errors, and abstentions, then
includes agreeing cases so review does not select only known failures.

```sh
npm run bench:triage -- --quality .cache/battle-001/quality.json --performance .cache/battle-001/performance.json --output .cache/triage-001
npm run bench:performance -- --retry .cache/battle-001/performance.json --output .cache/retry-001.json --runs 1 --samples 1 --warmup 0 --timeout-ms 45000
npm run bench:triage -- --quality .cache/battle-001/quality.json --performance .cache/battle-001/performance.json --retry .cache/retry-001.json --output .cache/triage-002
```

Use the original report's run count when replaying failures. A filtered replay
cannot pass the full-input latency screen. Triage emits three content-free files:

- `triage.json`: case IDs, symptoms, review priority, and replay classifications.
- `annotations.json`: independent policy reviews, initially `unresolved` with
  `reviewed: false`. Empty unresolved spans are not clean labels.
- `partitions.json`: diagnostic and reserved evaluation IDs, keeping connected
  exact-text duplicates and upstream families in the same partition.

Repeat `--retry FILE` on `bench:triage` to retain multiple sequential replays
of the same original performance report. Every replay must bind that report's
hash. Mixed failures and successes are classified as `intermittent_completion`,
not repaired failures. `review_focus` orders priority cases by identity false
positives, contextual false positives, evasion misses, and abstentions, without
changing their unresolved review status.

Reviewers consult the licensed local corpus, specify intended spans in UTF-16
units, and record their identity, date, policy version, and classification.
Classifications are `policy_mismatch`, `detection_false_positive`,
`detection_false_negative`, `span_boundary_error`, `expected_abstention`,
`unexpected_abstention`, `aligned`, or `unresolved`. Set `reviewed: true` and
`review_status: reviewed` only after approval; unresolved cases cannot be
approved. Changing an existing overlay requires a new review artifact rather
than applying conflicting judgments together.

The reserved partition is conservative, not a guaranteed representative
holdout. Scunthorpe families are category-level, so a diagnostic sample can
consume most available English families. Check `reserved_english` before
claiming independent English evaluation coverage. Obtain additional reviewed
families if none remain; do not weaken grouping to manufacture a holdout.

Candidate tuning waits for reviewed cases. Reuse `bench:decisions` for lexical,
CRF, and accelerated BERT experiments and the existing
`scripts/qualify-decision-quality.mjs` gate. Keep the current default when
reviewed quality, parity, or complete performance evidence is missing. The
external diagnostic cases remain evaluation-only, not training data.

## Replacement and end-to-end checks

The suite exercises default `fixed_phrase`, a custom phrase, and `omit`.
Golden synthetic fixtures check exact outputs independently of model detection,
including Unicode, multiple spans, protected code, quotations, URLs, invalid
offsets, and boundaries through 64 KiB. Native `rewrite` is also compared with
`detect` plus Rust `applyReplacements` on the selected external cases.

CLI checks cover successful rewrites, clean inputs, protected abstentions,
invalid requests/configuration, and unavailable models. Errors must omit source
text. Installed-host checks observe successful sanitized submissions for each
strategy, opt-in metrics, and absence of original text in observed provider
payloads. Host checks also cover real protected-span and unavailable-model
errors, plus timeout responses and worker exits injected into a temporary
plugin copy. Recovery retries the retained draft after repair; protected text
requires editing first. No failure switch is added to the shipped runtime.

Installed-host evidence uses a versioned, content-free event stream. Each
scenario has its own checkpoint beside the E2E output, and validated events
are appended as they arrive. A failed process retains its last observed phase
and any verified absence of provider requests; missing completion evidence
never becomes a pass. Historical reports without these fields remain readable.

After a refusal, the harness requests a terminal redraw and waits up to ten
seconds to observe the retained synthetic draft at the prompt box before
pressing Enter. Rendering whitespace is normalized only for this readiness
check; the provider payload must still contain the exact expected replacement.
Terminal output stays in memory. A readiness timeout is `draft_not_observed`,
not proof of draft loss. Recovery stalls and non-reproduced failures remain
visible until their cause is established.
Injected timeout responses test adapter handling, not the model deadline or
the host's hard process watchdog. Provider traffic is observed through recovery.
The adapter restores the draft only into an empty prompt box for the latest,
uncancelled submission; it does not overwrite a newer draft. A host refusal
alone does not imply that the prompt box retained the draft.
Existing plugin tests separately exercise retained drafts on failures.
This evidence does not prove the absence of host telemetry or other submission
paths.

Synthetic minimal pairs in `tests/fixtures/context-probes.json` exercise
identity context, negation, quotation, reclamation, substrings, usernames,
spacing, leetspeak, and Unicode obfuscation. Their content-free observations
are diagnostic, with unresolved semantic expectations; only preservation and
expected error handling are implementation checks.

## Performance interpretation

The default screen requests three runs, two warmups, and ten samples for every
existing synthetic fixture through 64 KiB, in both strategies and both native
and Node-API boundaries. The worker processes measure resident rewrites after
loading; load and whole-process elapsed time remain separate. Latency workers
do not run a second inference for profiling. `bench:profile` measures stages
in separate processes and reports token counts, planned/completed windows,
encoder time, and replacement time, including numeric progress on failure.
Its results cannot invalidate or substitute for latency samples. Each fixture uses
a fresh process, and warmups apply to that process. Runs alternate strategy order.

Use a smaller pilot before expensive runs:

```sh
npm run bench:performance -- --output .cache/pilot-001.json --runs 1 --samples 1 --warmup 0
```

`--timeout-ms` bounds each worker (default 120000 ms). A worker timeout is a
censored measurement, not a completed latency sample. Completed samples are
streamed to `.samples.jsonl`, and every worker outcome to `.progress.jsonl`.
The final report retains completed samples even if a later sample times out.
Runtime errors retain their measured duration but never count as successful
throughput. Failure phase and cause distinguish load, warmup, rewrite, report,
runtime timeout, worker timeout, exit, and malformed/incomplete evidence.
Raw worker output is never persisted.

The model's 30-second timeout is cooperative: it is checked between window
batches and after inference, not preemptively inside an encoder call. Input
byte length alone does not predict cost; token density and window count matter.
Neither the model timeout nor the 64 KiB input limit is raised for benchmarks.

Quantiles with small sample counts are descriptive observations, not tail
guarantees. Reports retain per-fixture distributions and matched strategy
comparisons, not pooled success-only distributions across different workloads.
An executed screen that misses the 500 ms target also returns nonzero; its
report can still have `status: completed` to distinguish complete measurement
from failed execution. The separate `under_500_ms` field records the target.

Screens never qualify a backend. The established qualification gate still
requires three 10,000-sample runs per fixture, at least 20 warmups, complete
native and Node coverage, and every observed resident rewrite below 500 ms.
Use `npm run bench:decisions -- --help` for the existing qualification protocol.
Shared CI hardware cannot qualify latency.

## Publishing the results

`bench:all` creates `quality.json`, `e2e.json`, `performance.json`, a compact
`snapshot.json`, and `report.md`. Reports record artifact hashes, hardware,
versions, configuration, source revision, and a dirty-worktree fingerprint.
Runs verify identities again on completion. Changing code or artifacts during
a run invalidates comparison evidence. Complete source/harness identities must
match across stages before assembling a report; do not edit code between runs.
They contain case IDs and counts, not dataset text or private prompts.

To assemble separately run stages, place the three named JSON reports in one
directory, then generate and publish the README block explicitly:

```sh
npm run bench:report -- --output .cache/battle-001
npm run bench:readme -- --output .cache/battle-001
```

Skip `bench:report` when `bench:all` already generated it. Existing report files
are never overwritten. `bench:readme` intentionally updates only the marked
README block and `benchmarks/results/latest.json`, after checking evidence
digests and compatible source, harness, model, machine, and runtime configuration.
It rejects changed-during-run or filtered performance evidence. Failed checks
remain visible. Review the resulting diff before committing.
