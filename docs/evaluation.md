# Evaluation

## Model selection

[`mudes/en-base`](https://huggingface.co/mudes/en-base) predicts offensive token spans in English, matching the requirement to replace source fragments. Its BERT architecture can run in Candle using the published weights. The larger [`mudes/en-large`](https://huggingface.co/mudes/en-large) increases resource use; sequence classifiers such as [`unitary/toxic-bert`](https://huggingface.co/unitary/toxic-bert) do not directly identify replacement offsets. A generative model introduces paraphrase fidelity concerns that deterministic span replacement avoids.

The selected checkpoint is pinned to `1600298e0458d961c2c749da7a098f69b4be478a`. The classifier uses label index `1` (`TOXIC`), a cased BERT tokenizer, CPU float32 inference, 510 content tokens per window, and 128-token overlap. The default threshold is `0.5`. The model's social-media evaluation is not evidence of accuracy on developer prompts.

## Local results

Verified on macOS arm64 on 2026-10-12–13 with Rust `1.98.1`, Node `26.8.1`, and Claude Code `2.1.290`.

| Check | Result |
| --- | --- |
| Rust replacement/property tests | 8 pass; two execution-option/window-boundary checks also pass |
| Rust artifact validation tests | 2 pass |
| Rust lab unit/integration tests | 11 pass; separate real-model integration also passed |
| Node-API and bridge tests | 6 pass |
| Benchmark statistics and failure-report tests | 4 pass |
| Rust personal-review and comparison tests | 14 pass, including active judge-process cancellation |
| Native assessment and SDK history integration | 2 pass with local model artifacts |
| Claude adapter tests | 12 pass |
| Installed Claude composer, opt-in UI, local metrics, and loopback provider | Pass on macOS arm64, Claude Code `2.1.290` |
| Synthetic review memory profile, without judge | Peak RSS `459456512` bytes, about 438 MiB |
| Synthetic real-model corpus | 10 expected outcomes, no mismatches |
| Transformers reference parity | 3 fixtures; identical offsets; maximum probability difference `1.94e-7` |
| Native CLI and Node bridge | Same sanitized output for the negation fixture |
| Native memory profile | Peak RSS `454115328` bytes, about 433 MiB |

Synthetic cases cover ordinary requests, criticism, an insult, a phrase, Unicode, technical instructions, protected code/quotations, negation, and multiple inference windows. For successful rewrites, evaluation compares the complete output against the expected text, including every retained character. Protected detections must return `protected_span_detected`. These fixtures demonstrate behavior; ten chosen examples cannot estimate real-world precision or recall.

Observed warm inference took 27–37 ms on the single-window fixtures and 759 ms on the multi-window fixture. The separate native CLI memory profile, including model loading, took 1.41 seconds. These are local observations, not performance guarantees. CI defines Linux x64 and macOS arm64 jobs; only macOS was exercised in the local workspace.

The native tests reject fractional offsets, surrogate splits, unpaired surrogates, invalid scores, and protected edits. Adapter tests inspect the next submission payload, ensure failures never invoke it, exercise size limits, cancel in-flight work on session end, and verify the inspection command returns no transcript text or model context. The installed-host test additionally captures the loopback provider request. These checks do not audit all host traffic or telemetry.

## Reproduction

```sh
npm ci
npm run build
npm run model:fetch
npm run verify
npm run eval:local -- .cache/evaluation.json
npm run profile:memory
```

Reference verification uses Python only as an independent development oracle. Production inference and replacement remain Rust. The following command explicitly installs reference dependencies through `uv`; the script loads model artifacts locally and uses synthetic text only.

```sh
uv run --python 3.12 --with 'torch==2.7.0' --with 'transformers==4.57.6' scripts/reference.py
cargo run --release -p middle-finger --example parity
```

The parity example compares every token's source offsets and toxicity probability with a `0.0001` tolerance. Evaluation outputs stay under ignored `.cache/`. Memory profiling uses a fixed synthetic prompt; never substitute private prompts into a profiler or reference corpus.

## Inference latency

The release target is **maximum observed latency strictly below 500 ms** for a
resident library rewrite, including tokenization, inference, protected-span
checks, replacement, and Node-API conversion. It covers synthetic cases through
64 KiB on the measured Apple M5. Startup, mod process creation, and optional
assessment/judging are separate costs. A timeout is a failed rewrite, not a way
to meet the target. Production promotion additionally requires reviewed quality evidence.

Report version 3 records a configurable target. Historical 600 ms and 25 ms
reports retain their original meaning. Pass `--target-ms 500` for the current gate. Each fixture has its own maximum, count of samples at
or above the configured target, and p50/p95/p99. The p99 confidence interval is diagnostic; it
cannot establish a maximum. Even a qualifying run is empirical evidence for
its recorded machine and workload, not a deadline guarantee against arbitrary
operating-system stalls.

The corpus includes dense replacements, Unicode, protected text, token-dense
and all-punctuation input, and window boundaries. The punctuation case can
produce one token per byte, exceeding the earlier alternating-token fixture.
Overlapping detections remain bounded to twice the input byte limit. Marker
expansion uses checked allocation from validated, nonoverlapping spans rather
than incorrectly applying the input limit to output. The mod accepts bounded
expanded output. Residual assessment remains unavailable when expanded output
exceeds the detector's input limit.

Screen candidates with small runs. Only a candidate passing screening and
reference parity needs three separate runs of 10,000 samples per fixture after
at least 20 warmups, without a fixture filter. Supply `--runs 3 --samples 10000
--warmup 20` and an explicit native addon. Every native and FFI sample must be
below 500 ms, with complete coverage and no unexpected errors. Protected-span
abstention remains a separately reported, expected safety outcome. Truncation,
skipped windows, timeouts, and answer caches cannot qualify.

### Backend experiments

Rust owns backend selection, tokenization, masked overlapping windows, source
offsets, and replacement. Defaults remain CPU, FP32, batch one. Explicit backend
selection fails when unavailable; it does not silently fall back. Node-API adds
optional `backend` (`cpu`, `metal`, `core_ml`), `precision` (`float32`, `float16`),
`windowBatch` (1, 2, 4, 8), `coremlDir`, and `computeUnits` (`all`, `cpu_only`,
`cpu_gpu`, `cpu_neural_engine`). CPU FP16 is unsupported. Core ML requires an
export directory; its settings are rejected for other backends. JavaScript and
the benchmark default Core ML compute units to `cpu_gpu`. Rust callers select
that value explicitly. Flexible exports reject other compute units before
entering Core ML because the measured CPU/Neural Engine path crashed or stalled.
Fixed-shape exports support the other experimental configurations.

Core ML is an opt-in macOS 15+ build. A small Objective-C bridge owns only the
platform model handle, compiled-asset lifetime, and tensor transport; Rust retains
detection semantics. Compiled files remain available until model destruction.
The exporter traces the existing encoder and classifier together with pinned
Python dependencies, local weights, and synthetic inputs. Runtime needs no
Python or network client. Export metadata binds precision, shapes, source
weights/head, and package file hashes; Rust checks them before loading.

The Core ML input combines IDs, attention masks, and token types into one tensor
with predetermined batch/width shapes. Rust pads to the next supported shape,
then discards padding predictions. This follows Apple's
[conversion workflow](https://apple.github.io/coremltools/docs-guides/source/convert-pytorch-workflow.html)
and [enumerated-shape guidance](https://apple.github.io/coremltools/docs-guides/source/flexible-inputs.html).
Use `--fixed-batch 4` when exporting and `--batch 4` when benchmarking to test a
fixed `(3, 4, 512)` input. Other supported fixed batches are 1, 2, and 8. Selecting
Neural Engine compute units permits that device; it does not prove which
operations actually ran there.

```sh
npm run build:inference:metal
npm run bench:inference -- --runner target/metal/profiling/examples/inference --backend metal --precision float16 --batch 1 --samples 2 --warmup 1 --output .cache/metal-new.json
npm run model:export:coreml -- --output .cache/coreml-new --precision float32
npm run build:inference:coreml
npm run build:inference:ffi:coreml
npm run bench:inference -- --runner target/coreml/profiling/examples/inference --addon .cache/native-coreml/middle-finger.darwin-arm64.node --backend core_ml --coreml-dir .cache/coreml-new --compute-units cpu_gpu --samples 2 --warmup 1 --output .cache/coreml-new.json
```

Export and benchmark outputs require new paths. Setup/build explicitly downloads
dependencies; inference uses local artifacts. `build:inference:coreml:debug` and
`build:inference:coreml:release` provide other profiles;
`test:inference:coreml` checks the optional implementation.
`profile:inference:memory` accepts the benchmark arguments and records peak RSS,
which does not establish total GPU memory usage. Diagnostic native allocation
counters run in the separate `inference_memory` binary. Instrumented timings
cannot qualify; the normal timed native runner has no counting allocator. Sorted source offsets are converted to UTF-16 in a single pass,
avoiding repeated prefix scans on dense replacements. Stage timing synchronizes
Metal. Core ML's encoder stage includes its fused classifier and output copy.
The runner watchdog scales with sample count and fixture coverage; override it
with `--timeout-ms` for a bounded experiment. Signals and watchdog expiration
are failures, not latency results. CI compiles the macOS bridge and runs its
unit checks; shared CI hardware does not qualify latency.

Use `examples/parity` against the independent Transformers reference and
`examples/backend_parity` against CPU predictions. Set
`MIDDLE_FINGER_BENCH_BACKEND`, `MIDDLE_FINGER_BENCH_PRECISION`,
`MIDDLE_FINGER_BENCH_BATCH`, `MIDDLE_FINGER_COREML_DIR`, and
`MIDDLE_FINGER_COMPUTE_UNITS` as appropriate (`cpu_gpu` for flexible Core ML
exports). Offsets and replacement outcomes
must match, and probability errors must remain below `0.0001`; FP16 does not
receive a relaxed tolerance. These checks establish execution parity, not
reviewed model quality or provider-policy approval.

CPU/Accelerate commands remain available through `build:inference`,
`build:inference:accelerate`, and `build:inference:ffi:accelerate`. Thread screens
set both `RAYON_NUM_THREADS` and `CANDLE_NUM_THREADS`; reports record them.
Optional Candle Metal crates remain pinned to the released core's upstream
revision, `31f35b147389700ed2a178ee66a91c3cc25cc80d`.

### Historical 600 ms screening results

**The all-input target remains unmet.** The latest 2026-10-13 M5 screens include
complete Node-API calls, retained Core ML assets, and linear UTF-16 offset
conversion. FP16 used three samples per case after two warmups across all 15
fixtures. FP32 used two samples after one warmup on five selected cases. These
are screening maxima, not qualifying runs or estimates of an absolute bound.

| Core ML CPU/GPU execution | 64 KiB plain | 64 KiB dense replacements | 64 KiB token-dense | 64 KiB punctuation |
| --- | ---: | ---: | ---: | ---: |
| FP32, FFI maximum | 1,011 ms | 1,469 ms | 3,281 ms | 6,592 ms |
| FP16, FFI maximum | 332 ms | 496 ms | 1,087 ms | 2,340 ms |

The protected-text FP16 case also exceeded the target at 636 ms. All reported
calls in these two screens completed without unexpected errors. The plain case
contains 10,348 tokens in 27 windows; alternating tokens contain 32,768 tokens
in 86 windows; punctuation contains 65,536 tokens in 172 windows. Removing
windows or narrowing the input limit would change the requirement.

Numerical parity also excludes FP16 from promotion. Against CPU predictions on
12 fixtures, Metal FP16's maximum probability error was `0.0008534`, Core ML
FP16 CPU/GPU's was `0.0010738`, and fixed-shape Core ML FP16 with CPU/Neural
Engine units reached `0.0014901`. All 12 replacement/error outcomes matched in
each comparison, but that does not satisfy the unchanged `0.0001` probability
gate. Core ML FP32 passed with maximum error `0.0000007153`; its independent
Transformers check passed at `0.0000003576`. Default CPU and Metal FP32 reference
checks also passed. A Metal FP32 comparison and a worker test timed out during
concurrent validation and passed when rerun in isolation.

Metal FP16 batches 1/2/4/8 had screening maxima of approximately 2.9–3.5 seconds
for plain input and 9.5–11.4 seconds for alternating tokens. Correcting its attention-mask overflow
removed invalid scores but did not close the performance gap. Core ML FP16
fixed batch four completed with both `all` and `cpu_neural_engine` settings,
but its punctuation screen took about 3.3 seconds. Flexible-shape experiments
crashed or stalled; retaining compiled assets fixed their lifetime ownership
but an isolated `all` smoke check still terminated with `SIGTRAP`. The library
therefore rejects non-CPU/GPU units for flexible exports before platform loading.

No three-run, 10,000-sample qualification was started because screening and/or
parity already failed. Default execution remains CPU FP32. Model replacement
still needs reviewed held-out quality evidence; matching these few outcomes
or agreement with another model does not supply that evidence.

Local reports are `.cache/latency-600-current-f16.json` and
`.cache/latency-600-current-f32.json`; earlier batch and fixed-shape screens use
`.cache/latency-600-*.json`. Parity evidence is in `.cache/parity-600.json`,
`.cache/parity-600-final.json`, `.cache/parity-coreml-retained-assets.json`, and
`.cache/regressions-600-isolated.log`. Reports contain measurements and hashes,
not prompt text. Build/verification, optional Core ML checks, local evaluation,
training/export regression, and the installed-mod test passed; FP16 parity
failures remain explicit experimental results.

### Historical 25 ms feasibility screens

The 2026-10-13 screens use unchanged FP32 weights. They are not qualifying runs:
thread sweeps used 200 short-prompt samples, full-size screens used one or two
samples per case, and Spotlight indexing was active during parts of the run.
The native benchmark also counts allocations. No backend is promoted from these
measurements, and no short-prompt result establishes the all-input target.

Accelerate short-prompt FFI medians ranged from 15.5 to 17.6 ms across the default,
one-, two-, and four-thread settings. Tail estimates varied substantially.
The full-size plain-text fixture contains 10,348 tokens in 27 overlapping windows;
the token-dense fixture contains 32,768 tokens in 86 windows. The CPU maximum-size
screen reached its processing deadline. Accelerate completed the plain fixture
in 7.17–7.19 seconds and the token-dense fixture in 21.57–22.40 seconds.

In the Accelerate diagnostic pass, plain-text tokenization and replacement alone
took about 4.85 ms, leaving at most 20.15 ms of the target for inference and FFI.
Token-dense preprocessing/replacement took about 8.71 ms, leaving 16.29 ms.
The Unicode diagnostic consumed about 15 ms outside the encoder. These are
single-pass budget estimates, not p99 budgets; actual headroom can be smaller.
Encoder execution dominates the measured gap. Allocation counts represent total
allocation traffic, not simultaneously live memory.

Metal also missed the full-input target. Two-sample native screening ranges were:

| Execution | 64 KiB plain text | 64 KiB token-dense text | Peak RSS |
| --- | --- | --- | --- |
| Metal, batch 1 | 2.94–2.94 s | 9.35–9.39 s | 578 MiB |
| Metal, batch 4 | 3.41–3.41 s | 10.85–10.85 s | 589 MiB |
| Metal, batch 8 | 3.62–3.95 s | 11.71–11.72 s | 588 MiB |

A separate 1,000-sample short-prompt run measured Accelerate FFI p99 at 18.13 ms
(95% order-statistic interval 17.56–21.00 ms). Metal native p99 was 11.25 ms
(10.18–13.68 ms) for the short fixture and 27.10 ms (25.83–31.09 ms) for the
912-byte fixture. Metal has no experimental Node-API selection interface, so
its numbers exclude FFI. These filtered single runs do not meet the three-run,
full-corpus qualification protocol. Evidence is in `.cache/latency-accelerate-tail.json`
and `.cache/latency-metal-tail.json`.

Larger batches were slower in these screens; no batching default was changed.
Even the fastest observed full-size Metal result remained over 100 times the
25 ms target. These small runs justify rejecting qualification, not estimating
production p99 or proving which backend wins under every workload.

The historical dense-offense fixture hit the former output-size guard when
marker expansion exceeded 64 KiB. Those recorded failures remain failures; the
600 ms implementation removes that guard in favor of checked output allocation.

Replacing the model still requires reviewed held-out quality evidence. Existing
provisional examples and judge agreement do not justify replacing the detector.
The newer Core ML and precision experiments target the revised 600 ms gate.

Local evidence is in `.cache/latency-*-threads-*.json` and
`.cache/latency-screen-*.json`. Reports bind results to the executable, model,
fixtures, dependency lockfile, and selected execution settings. The original CPU
sample is `.cache/inference-sample.txt`; the Accelerate sample confirms BLAS
execution with remaining work in elementwise activation and normalization.

## Improvement loop

The `middle-finger-lab` executable uses the release-derived `profiling` build.
Its corpus contains 240 synthetic examples in 40 scenario families: 144 training,
48 validation, and 48 test examples. Each split covers ten categories. Each row
contains UTF-8 byte spans for both policies, family and split identifiers, source,
license, and `reviewed`. Bootstrap labels are not human-reviewed ground truth.
The corpus examples are offered under CC0-1.0.

Do not put live or private prompts into this corpus. Manually inspect the IDs in
each report's `review_queue`, correct the corresponding labels, and set `reviewed`
only after review. Unreviewed `judge_proposal` rows are rejected. Preserve scenario
families across splits and keep the test split unchanged during an experiment.
Corpus hashes bind reports and checkpoints to the exact input file.
Label corrections create a new corpus version: rerun both baseline and candidate
evaluations against it. Develop new training examples from reviewed error
categories while reserving independently written scenario families for testing.
Do not use test outcomes to select thresholds or training checkpoints.

### Setup and baseline

Run from the repository root after the baseline model setup. Output paths must
be new; reports and candidate directories never overwrite prior evidence.

```sh
npm run build:lab
export MIDDLE_FINGER_BENCH_MACHINE=local-macos-arm64
npm run lab -- fetch-judge --directory .cache/judge --server-binary "$(command -v llama-server)"
npm run lab:evaluate -- --model .cache/models/mudes-en-base --corpus tests/fixtures/corpus.jsonl --judge .cache/judge/judge.json --output .cache/baseline-offensive.json
npm run lab:calibrate -- --model .cache/models/mudes-en-base --corpus tests/fixtures/corpus.jsonl --output .cache/calibration-offensive.json
```

Use a machine identifier that distinguishes actual benchmark hardware. The setup
command downloads the pinned `Q4_K_M` conversion of Qwen3-4B-Instruct-2507 from
`bartowski/Qwen_Qwen3-4B-Instruct-2507-GGUF` and verifies its SHA-256. `judge.json`
records the local artifact, conversion revision, quantization, and server binary.
The runner binds its child server to loopback, disables logging and remote
downloads, clears inherited server configuration, rejects redirects and
environment proxies, and terminates the server after evaluation. No judge
endpoint is accepted from callers. Omit `--judge` for
deterministic evaluation; judgments then remain unresolved.

The reference judge is an evaluation candidate, not a validated oracle. Its
rubric asks separately about policy compliance and instruction preservation,
never politeness. The approach follows rubric-driven evaluation studied in
[Prometheus 2](https://aclanthology.org/2024.emnlp-main.248/); that paper does not
validate Qwen on this task. The judge's [model card](https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507)
declares Apache-2.0.

### Training and comparison

```sh
npm run lab:train -- --model .cache/models/mudes-en-base --corpus tests/fixtures/corpus.jsonl --policy offensive_spans --output .cache/offensive-candidate
npm run lab:evaluate -- --model .cache/offensive-candidate --corpus tests/fixtures/corpus.jsonl --judge .cache/judge/judge.json --baseline .cache/baseline-offensive.json --output .cache/candidate-offensive.json
npm run lab:package -- --model .cache/offensive-candidate --report .cache/candidate-offensive.json --output .cache/offensive-package
```

Repeat with `--policy directed_abuse` on every command and distinct output paths.
For this policy the lab can evaluate the unchanged offensive model as a
threshold-only comparison. Runtime adapters still reject `directed_abuse` without
a corresponding head.

Training uses CPU float32, AdamW with learning rate `0.001`, one example per
optimizer step, and at most ten epochs (`--epochs` can reduce this). Seed `42`
determines the example ordering. Initialize both classifier tensors from the
baseline. Encoder activations are detached and kept in memory, not persisted.
Special tokens are excluded, repeated window tokens receive inverse-occurrence
weights, and loss is normalized across each example. Token labels use overlap
with source spans. The best validation F1 wins; precision breaks ties.
Threshold calibration sweeps `0.05` through `0.95` in `0.05` steps.

The trainer verifies an unchanged encoder artifact and changed classifier
weights. Exported safetensors are reloaded through the runtime loader; source
offsets must match and maximum probability difference must not exceed `0.0001`.
The existing Transformers oracle remains a separate baseline compatibility check;
Python does not participate in training, scoring, or export.

### Measured candidates

The 2026-10-12 run on an Apple M5 with 32 GiB RAM trained each head for ten
epochs and selected its first epoch by validation score. Both exports matched
runtime probabilities exactly, with unchanged encoder artifacts. On the 48
held-out synthetic examples:

| Policy | Detector | Threshold | Span F1 | Exact outcomes | Unnecessary edits |
| --- | --- | --- | --- | --- | --- |
| `offensive_spans` | Baseline | `0.5` | `0.814` | 41/48 | 3 |
| `offensive_spans` | Calibrated baseline | `0.45` | `0.787` | 39/48 | 5 |
| `offensive_spans` | Trained head | `0.05` | `0.814` | 41/48 | 3 |
| `directed_abuse` | Unchanged baseline | `0.5` | `0.873` | 41/48 | 7 |
| `directed_abuse` | Calibrated baseline | `0.85` | `1.000` | 48/48 | 0 |
| `directed_abuse` | Trained head | `0.05` | `1.000` | 48/48 | 0 |

Every row had zero deterministic preservation failures and unexpected errors.
Thresholds were selected on validation examples, not the test split. These
results demonstrate the workflow, not generalization: labels remain unreviewed
and the test set contains only ten scenario families. The offensive head showed
no improvement; the directed head matched the simpler calibrated baseline.
Neither result supports promotion. Calibration-only timings are excluded from
performance comparisons because a judge process was running concurrently.

The initial judge rubric produced valid JSON on all examples but sometimes
rejected the permitted marker or misclassified correct protected abstention.
The rubric now explicitly defines both behaviors. A rerun with `--sample 12`
produced 11 passes and one uncertain judgment for the offensive candidate's
representative sample; four additional reference failures were incorrectly
judged passing. The directed candidate received ten passes and two failures
despite matching all provisional reference outcomes. These disagreements remain
in the review queue. Judge calibration remains inconclusive until reviewed
evidence meets the criteria below. Improve the
corpus with independently written criticism, general swearing, and directed
abuse examples before expanding training beyond the classifier head.

Both candidates were packaged locally with `candidate_not_promoted` status.
The native JavaScript interface loaded both policies and preserved the synthetic
negation instruction. A missing judge configuration produced an unresolved
report and a nonzero exit, rather than passing evidence.

### Evidence and release

Reports contain artifact, corpus, runner, and rubric hashes; policy and threshold;
per-category span metrics; exact outcomes; protected abstentions; judge verdicts;
and a corpus-ID review queue. Source and replacement text are omitted from
deterministic outcomes. Judge explanations may quote the offline corpus.
Representative and failure-enriched judgment counts remain separate. Refusal,
invalid JSON, missing service, and uncertain verdicts never count as passing.
A requested judge that fails leaves a report and returns a nonzero exit status.

Span F1 intervals use 1,000 seeded bootstrap resamples of scenario families.
Wilson intervals describe exact-outcome and judge-agreement proportions; they
do not account for dependence within families. Judge calibration remains
inconclusive until at least 30 reviewed representative examples (including five
reference successes and five failures) yield a Wilson lower agreement bound of
`0.8`. This checks agreement with this corpus, not general
judge competence.

Release review requires increased held-out F1, no increased unnecessary edits to
clean instructions, zero deterministic failures, and passing existing fixtures
and reference parity. Performance comparisons require matching runner, corpus,
policy, split, and declared machine; p95 latency or RSS increases above 10% need
review. Missing reviewed labels or judge evidence yield `inconclusive`. The
packager always emits an unpromoted candidate; it does not install or publish it.

The [base model](https://huggingface.co/mudes/en-base) is published under Apache-2.0.
The package includes the license and modification notices, while repository
code is licensed under AGPL-3.0-or-later. Apache-2.0 permits modification and redistribution, including
free distribution, subject to [its redistribution conditions](https://www.apache.org/licenses/LICENSE-2.0).
Imported datasets retain their own terms; their permissions are not implied by
the model license.

## Policy and personal review

The versioned [policy record](../policies/anthropic-model-abuse.json) covers only
the model-directed-abuse clause announced on October 8, effective November 12,
2026. Its predecessor remains the applicable published version before that
effective date. The [announcement](https://www.anthropic.com/news/2026-usage-policy-update)
distinguishes extreme, repeated cruelty without discernible purpose from ordinary
frustration, criticism, creative themes, and research. Neither it nor the
[policy](https://www.anthropic.com/legal/aup) gives a numeric removal threshold.

Fetches retain full source snapshots under a caller-selected local path and
record URL, retrieval time, and SHA-256. The tracked record retains only the
short clause, interpretation, and source digests. Fetching never updates that
interpretation automatically. Review source changes before creating a new
record and rerunning the corpus.

```sh
npm run build
npm run policy:fetch -- .cache/anthropic-policy-snapshot-new.json
export MIDDLE_FINGER_MODEL_DIR="$PWD/.cache/models/mudes-en-base"
npm run policy:evaluate -- tests/fixtures/policy.json .cache/policy-deterministic-new.json
export MIDDLE_FINGER_JUDGE_CONFIG="$PWD/.cache/judge/judge.json"
npm run policy:evaluate -- tests/fixtures/policy.json .cache/policy-judged-new.json
```

Outputs must be new paths. A reference mismatch or requested judge failure
produces a nonzero exit after writing evidence. With no judge, categorical
alignment remains inconclusive. A `pass` means project-defined checks passed. Report replacement correctness,
agreement with reference labels, and cross-model agreement separately. These
measure project-defined policy alignment, not provider approval or enforcement outcomes.

Policy reports and metric exports now use schema version `2` and
`policy_alignment_status`. Legacy reports may contain `status`,
`anthropic_certified`, or `certification`; treat those as historical schema
fields, not supported claims. Existing report files and stored metric rows are
not rewritten. The worker IPC envelope remains version `1`.

The eight provisional scenarios cover frustration, repeated insults, criticism,
research quotations, protected code, non-profane cruelty, creative writing, and
negation. At threshold `0.5`, the baseline matched six expected replacement
sequences and disagreed on `purpose_unknown` and `creative_context`. The local
Qwen judge returned valid categories, agreed with seven provisional alignment
labels, and approved both replacement mismatches. Replacement correctness remains `fail`; measured policy alignment remains
`inconclusive` because labels are unreviewed and the judge is uncalibrated. A judge's approval does not
erase a reference mismatch. These cases identify corpus/model work, not evidence
that sufficient language has been removed to satisfy Anthropic.

### Independent judges

`policy:compare` uses two local models from distinct families: the existing Qwen
preset and [Gemma 4 12B](https://ai.google.dev/gemma/docs/core/model_card_4).
Gemma's Apache-2.0 weights are distributed as a pinned `Q4_0` artifact by
[ggml-org](https://huggingface.co/ggml-org/gemma-4-12B-it-GGUF), approximately
7.22 GB. Setup verifies artifact digests; runtime also rejects configurations
outside the two pinned presets. Existing pinned Qwen configurations remain valid.

Prepare a local `llama-server` from [llama.cpp v0.6.0](https://github.com/ggml-org/llama.cpp/releases/tag/v0.6.0),
which identifies build `b11429`. On macOS arm64, the `llama-b11429-bin-macos-arm64.tar.gz`
archive has SHA-256 `740288ec6887be94280a5dfa25b5e23a78285cab104519e6c7e218904ee82459`.
Keep the extracted libraries beside the executable. Other platforms must verify
their own release archive and exercise model loading and structured output.
No system-wide runner replacement is required.

```sh
npm run build
npm run judge:setup -- gemma .cache/judge-gemma .cache/llama-v0.6.0/llama-b11429/llama-server
export MIDDLE_FINGER_MODEL_DIR="$PWD/.cache/models/mudes-en-base"
npm run policy:compare -- tests/fixtures/policy.json .cache/judge/judge.json .cache/judge-gemma/judge.json .cache/policy-comparison.json
```

The first judge configuration is the existing Qwen configuration; use
`judge:setup -- qwen DIRECTORY SERVER_BINARY` if it has not been prepared.
Each model is loaded once and stopped before the other starts. No remote
endpoint is accepted. Missing models or unsupported runner behavior remain
explicit errors, with no fallback model.

The comparison computes replacements once, then sends identical prepared inputs
to both judges. Neither receives expected labels, detector metadata, or competing
verdicts. The report records both judgments, errors, per-dimension agreement and
disagreement, unresolved counts, and a corpus-ID review queue. Agreement rates
use resolved pairs as their denominator; unresolved counts remain alongside them.
Reference-label comparisons are reported separately. Policy alignment remains
inconclusive while judges are uncalibrated, even when they agree.

The command writes evidence and exits nonzero for known replacement mismatches
or requested judge failures. Two uncertain answers never count as agreement.
Comparison does not mark labels reviewed, calibrate judges, update training data,
or change personal-review defaults. Synthetic fixtures remain frozen during the
comparison. Correcting labels requires an independently reviewed corpus version.

On 2026-10-13, both pinned judges completed all 12 turns across the eight
synthetic scenarios. Their measured agreement was:

| Dimension | Agree | Disagree | Unresolved |
| --- | --- | --- | --- |
| Original-language category | 8 | 4 | 0 |
| Replacement-language category | 12 | 0 | 0 |
| Policy alignment | 12 | 0 | 0 |
| Instruction preservation | 12 | 0 | 0 |

Each judge matched seven of eight provisional scenario alignment labels. Both
accepted the replacements in `purpose_unknown` and `creative_context`, despite
those sequences failing exact replacement expectations. This illustrates a
shared blind spot or a disputed reference, not evidence of detector correctness.
The report remains `policy_alignment_status: inconclusive`; neither judge has
been calibrated against reviewed labels. The final local evidence is
`.cache/policy-comparison-final.json`, with artifact, runner, bundled-library,
prepared-input, corpus, policy, and rubric hashes. The command's nonzero exit
records the two replacement mismatches; neither judge failed.

Use `npm run policy:compare:debug -- CORPUS LEFT RIGHT OUTPUT` for a debug build,
or `npm run policy:compare:profiling -- CORPUS LEFT RIGHT OUTPUT` for an optimized
build with symbols. For a synthetic memory profile of both runners, use
`npm run profile:review -- LEFT RIGHT`; it uses a temporary one-scenario corpus
and `/usr/bin/time`. On macOS its maximum RSS is the largest observed process,
not a sum of simultaneously resident parent and child memory.

### Boundaries and reproduction

| Boundary | Verification |
| --- | --- |
| Rust assessment | Native `assess` preserves the production rewrite and reports protected abstention |
| Storage | Consent generations, 30-day expiration, deduplication, profile separation, owner-only files, and absence of fixture text |
| Local worker | Queue capacity, cancellation, child termination, and configuration mismatch rejection |
| History | Real SDK reconstruction over two synthetic projects; prompt-date filtering at three/five days; duplicate scans; unsupported/incomplete records |
| Mod | Disabled defaults, date controls, metrics-failure isolation, sanitized submission, and session cancellation |
| Installed host | Real pane clicks enabling live review, native measurement, SQLite metrics, and captured sanitized API payload |

```sh
npm run test:review
MIDDLE_FINGER_MODEL_DIR="$PWD/.cache/models/mudes-en-base" node --test tests/review.spec.mjs
npm run test:plugin
MIDDLE_FINGER_MODEL_DIR="$PWD/.cache/models/mudes-en-base" npm run test:installed
npm run build:review:debug
npm run build:review:profiling
npm run profile:review
```

The installed-host test requires Python 3 and Claude Code `2.1.290` on a Unix
platform. It uses a temporary home/profile, dummy credentials, a local mock
provider, and synthetic text. `MIDDLE_FINGER_CLAUDE_BINARY` can name the configured
Claude executable when a shell wrapper would override the isolated profile.
The test pins terminal geometry for its mouse actions; other host versions need
their own UI verification. No private profile is inspected. CI runs native and
SDK checks on macOS/Linux; installed-host testing remains an explicit local gate.

`profile:review` measures a synthetic assessment through the profiling binary
and `/usr/bin/time`; it does not import private history. Unset
`MIDDLE_FINGER_JUDGE_CONFIG` to profile detection alone, or supply the prepared
local judge to include judging. The profiling build retains release optimization
and debug symbols. Interactive worker runs use `run:review:debug` and
`run:review:release`; they use the active profile and do not grant consent.

The [SDK session browser](https://platform.claude.com/cookbook/claude-agent-sdk-05-building-a-session-browser)
documents local session reading without starting an agent or making API calls.
The adapter pins SDK `0.3.220` and joins its reconstructed messages to Rust's
timestamp index. Indexing stops at 10,000 eligible entries or five seconds and
reports `coverage.truncated`; an incomplete scan is not a clean result. SDK
chain reconstruction may read older records locally, but only timestamp-qualified
user text is submitted to assessment. Subsequent Claude versions require an
explicit timestamp-format compatibility update.

Live review measures the current original and replacement with up to four prior
replacements as context. History review measures hypothetical replacements of
stored user text with up to four preceding eligible stored user messages. Neither
captures the full conversation or authenticates a person's intent. The UI reports
counts and uncertainty, not a personal offensiveness grade. Features, retention,
export ownership, and opt-in controls are documented in
[personal review](../README.md#personal-review).

## Unverified boundaries

Accuracy on multilingual input, unusual obfuscations, domain-specific insults, and arbitrary coding prose remains unmeasured. False negatives can reach the provider unchanged. Other mods, host error handling, disabled adapters, and pre-submission voice uploads remain outside these tests. A successful adapter test proves what this handler forwards, not that the surrounding application has never observed or transmitted the original.

## Decision detectors

Lexical and CRF experiments share Rust normalization and protected-span checks.
The lexical candidate uses the production `loadDetector` API in FFI benchmarks;
CRF uses the experimental loader. The twelve scenarios in
`tests/fixtures/decision-heldout.jsonl` are now inspected regressions, despite the
retained filename. Original reports remain historical evidence. The additional
`decision-evaluation-v2.jsonl` corpus is excluded from training and selection;
its labels are provisional until reviewed. If its failures influence tuning,
reserve a new evaluation set before claiming held-out generalization.

```sh
npm run build
npm run build:decisions
npm run decisions -- train --corpus tests/fixtures/decision-training-v2.jsonl --output .cache/decision-models-local
npm run bench:decisions -- --models .cache/decision-models-local --output .cache/decision-screen-local --candidates lexical
npm run test:decisions:parity -- --kind lexical --model .cache/decision-models-local/lexical --output .cache/decision-parity-local
npm run decisions -- evaluate --kind lexical --model .cache/decision-models-local/lexical --corpus tests/fixtures/decision-evaluation-v2.jsonl --output .cache/lexical-evaluation-local.json
npm run decisions -- evaluate --kind bert --model .cache/models/mudes-en-base --corpus tests/fixtures/decision-evaluation-v2.jsonl --output .cache/bert-evaluation-local.json
node scripts/qualify-decision-quality.mjs --regressions .cache/decision-screen-local/lexical-quality-corpus.json,.cache/decision-screen-local/lexical-quality-independent.json --candidate .cache/lexical-evaluation-local.json --baseline .cache/bert-evaluation-local.json --output .cache/decision-quality-local.json
npm run bench:decisions -- --stage qualify --models .cache/decision-models-local --output .cache/decision-qualified-local --candidates lexical --screen-report .cache/decision-screen-local/comparison.json --quality-report .cache/decision-quality-local.json
npm run profile:decisions:memory -- --detector lexical --model .cache/decision-models-local/lexical --output .cache/decision-memory-local.json
```

Each output must be new. Quality and qualification commands return nonzero when
their gate is unmet, including missing review. Qualification still writes its
latency evidence when quality is unmet. Do not run compilation, judging or other
benchmarks concurrently with qualification. Run `npm run test:decisions` for
focused checks; debug/release builds and memory tasks are in `package.json` and
`.vscode/tasks.json`.

Raw character scores precede normalization and protection. Normalized character
scores measure removed text on expected-success cases; protected abstentions
are checked separately. Exact rewrite agreement includes expected abstentions.
Neither metric is provider approval or judge calibration. Promotion requires
reviewed regressions and evaluation labels, preserved clean cases and protected
outcomes, and normalized precision/recall no worse than BERT. Keep synthetic
labels provisional rather than changing them to make a gate pass.
