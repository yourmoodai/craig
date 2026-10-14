# Craig

<img src="docs/assets/craig.png" width="240" alt="Craig in a blue hat, looking unimpressed and raising his middle finger">

**Another AI library. This one protects the chatbot from your vocabulary.**

Craig is a deliberately rude mascot for a useful local profanity and offensive-language filter. Named after Craig from *South Park*, this yourmood.ai project arrives with the obligatory mascot and a practical job: replace detected offensive spans before a Claude Code composer submission continues. The mascot supplies the gesture; the mod does not generate insults or banter.

```text
You idiot, do not delete the test.
        ↓ local classification and replacement
You ya ya ya, do not delete the test.
```

Write the frustrated instruction. Craig swaps detected spans for a configurable marker, defaulting to `ya ya ya`, and preserves the surrounding text exactly. A Rust library performs inference locally and offline; its typed JavaScript interface, CLI, and Claude Code mod share the same replacement core. Protected-span conflicts and processing failures stop submission rather than forwarding the original as a fallback. Slash commands and non-composer submissions bypass the mod.

**Freedom of speech == freedom of instruction.** We consider a vendor policing how politely you address software an Orwellian etiquette exercise. Craig is our tongue-in-cheek response: a profanity filter wearing a middle finger, for people who want to keep using the tool without sending it every word of their frustration.

Anthropic's [October 8 policy announcement](https://www.anthropic.com/news/2026-usage-policy-update) prohibits “sustained and needless abusive or cruel behavior toward our models,” effective November 12, 2026. It explicitly excludes ordinary frustration, pushback, dark creative themes, and testing or research. Craig is intended to help users remove language they do not want to submit under that [policy](https://www.anthropic.com/legal/aup). It does not establish compliance: detecting offensive words cannot determine whether a conversation meets a contextual behavior rule. Our evaluations measure policy alignment, not provider approval.

## Installation

Requires Rust, a C/C++ toolchain, Node.js 22.18 or newer, and Claude Code with mods support for the adapter. The adapter is tested with Claude Code `2.1.290`. Native build targets are macOS arm64 and Linux x64; build on the target platform.

```sh
npm ci
npm run build
npm run model:fetch
export CRAIG_MODEL_DIR="$PWD/.cache/models/mudes-en-base"
node bridge/cli.mjs doctor
claude --plugin-dir "$PWD"
```

To register the repository as a Claude Code marketplace and install the plugin (`craig@yourmood.ai`) after building the native addon and fetching model artifacts locally:

```sh
claude plugin marketplace add "$PWD"
claude plugin install craig@yourmood.ai
```

When hosted on GitHub (`yourmoodai/craig`), users with a built checkout or marketplace access can add the catalog via `claude plugin marketplace add yourmoodai/craig`.

Setup explicitly downloads the pinned model from Hugging Face. Inference makes no network requests. The default English classifier is [`mudes/en-base`](https://huggingface.co/mudes/en-base), revision `1600298e0458d961c2c749da7a098f69b4be478a`. Its Apache-2.0 model license is separate from this repository's AGPL-3.0-or-later license. Model weights are not committed or included in the npm package; the lab can explicitly package a local candidate with its licenses and evidence. Setup and Rust loading verify artifact hashes.

[`models/catalog.json`](models/catalog.json) is the source of truth for model identity, publisher and quantizer provenance, license, digest, and permitted role. `mudes/en-base` is the only submission-path detector. Qwen and Gemma are explicit-download, offline judges for personal review and evaluation; they cannot be selected as delivery detectors, generate prompt rewrites, establish provider approval, or certify policy compliance.

Set these environment variables before starting Claude:

| Variable | Meaning |
| --- | --- |
| `CRAIG_DETECTOR` | `bert` by default; opt-in `lexical` requires a decision artifact |
| `CRAIG_MODEL_DIR` | Required absolute path to downloaded artifacts |
| `CRAIG_THRESHOLD` | Override the model threshold; baseline default `0.5`, greater than zero and at most one |
| `CRAIG_POLICY` | `offensive_spans` by default; `directed_abuse` requires a matching candidate head |
| `CRAIG_REPLACEMENT` | Fixed marker, default `ya ya ya`; 1–80 non-control characters |
| `CRAIG_JUDGE_CONFIG` | Optional absolute path to a locally prepared `judge.json`; used only after personal review is enabled |
| `LOG_LEVEL` | `silent` suppresses diagnostic error codes; other values retain them |

The mod rewrites composer submissions. `/craig` opens a local inspection pane with the latest original and replacement; it does not add them to model context. Inspection data is cleared at session start/end. Slash commands and non-composer submissions bypass this adapter.

## Migrating from middle-finger

Craig is a clean rename: the command is `/craig`, the CLI is `craig`, and
environment variables use `CRAIG_` instead of `MIDDLE_FINGER_`. Old names are
not aliases. The marketplace is `yourmood.ai`; GitHub ownership remains
`yourmoodai`.

Before switching, disable both personal-review capabilities in the old mod,
uninstall `middle-finger@yourmoodai`, close its sessions, and stop its review
worker. Build Craig and follow the setup above to install
`craig@yourmood.ai`. Reconfigure environment variables before launching Claude.
Existing model directories remain usable; weights need not be downloaded again.

To retain metrics, first confirm both old and new workers are stopped and that
`<active-profile>/craig` does not exist. Move the entire
`<active-profile>/middle-finger` directory to `<active-profile>/craig`; never
merge or overwrite an existing destination. Disabling review before this move
preserves the metrics while clearing consent. If that was not done, leave the
old directory untouched and start with fresh metrics instead. Craig performs
no automatic migration. Review capabilities must be enabled explicitly again.

Historical fixture provenance and benchmark reports retain their original
names and artifact hashes. Rebuilding under the Craig name does not requalify
those performance measurements.

## Personal review

Open `/craig`, select **personal metrics**, and enable the capabilities
you want. Both start disabled:

- **Live metrics** measures each processed composer prompt and independently
  samples approximately 10% for the local judge, capped at 100 samples per UTC day.
- **History access** permits an explicit scan across projects in the active
  `CLAUDE_CONFIG_DIR` profile. The date selector defaults to three days and allows
  one through five days. Rust filters each prompt's timestamp, not its file's
  modification time. History samples approximately 10% for judging.

The pane exposes refresh, cancellation, export, and deletion. Metrics contain
counts, removed proportions, classifier confidence, and categorical judgments;
confidence is not a measure of severity. Judge setup is explicit through the
[evaluation workflow](docs/evaluation.md#setup-and-baseline). Missing judge
artifacts leave sampled results unresolved. No remote judge is used.

Only numerical and categorical metrics and provenance hashes are stored in
`<active-profile>/craig/metrics.sqlite3`, with owner-only permissions.
Originals, replacements, excerpts, session titles, and judge explanations are
not persisted by this feature. Retention is 30 days, enforced when the worker
starts or metrics are read; no purge runs while it is closed. Exported files
are user-managed copies outside automatic retention and deletion.

History describes text Claude already stored. It cannot recover originals
replaced before submission or prove who authored a stored user message. The
timestamp adapter currently recognizes Claude Code `2.1.290`; unsupported,
missing, or malformed records appear in coverage counts. Live and history
measurements remain separate. Scans retry cancelled or failed assessments;
completed observations are deduplicated within the same worker configuration. Private history never becomes training data.

The worker queues at most four jobs plus one active job. Disable or cancel
discards queued text and terminates active judging; deletion also prevents late
writes. Detection already running may finish in memory before releasing its
input. A metrics failure does not restore an original submission. The worker
exits after 60 idle seconds. After changing model, marker, threshold, or judge
configuration, stop the previous worker before reopening the pane:

```sh
printf '%s' '{"command":"shutdown"}' | npm run --silent review
```

The worker transport currently supports macOS and Linux. Other agents can reuse
Rust assessment and storage behind their own history and UI adapters.

## Measured policy alignment

The [versioned policy record](policies/anthropic-model-abuse.json) identifies the
announced model-abuse clause, effective date, source digests, and this project's
interpretation. Explicit fetch and evaluation commands are in
[evaluation](docs/evaluation.md#policy-and-personal-review).

Reports distinguish reference failures, unresolved judgments, and project-defined
measured policy alignment. Replacement correctness, agreement with reference
labels, and cross-model agreement are separate measurements. None establishes
provider approval or predicts enforcement. Provisional labels and unresolved
judgments remain visible.

The offline `policy:compare` command runs Qwen and Gemma independently against
identical synthetic inputs. Neither sees the expected labels or the other
judge's answers. See [independent judges](docs/evaluation.md#independent-judges)
for setup and reproduction.

## Library

```ts
import { loadDetector, applyReplacements } from 'craig';

const detector = await loadDetector({ modelDir: '/absolute/model/path' });
const result = await detector.rewrite('You idiot, fix the test.');
console.log(result.text);

const measured = await detector.assess('You idiot, fix the test.');
// No text is returned when protected material requires abstention.
console.log(measured.assessment.replacementCount);

const detection = await detector.detect('You idiot, fix the test.');
const custom = applyReplacements('You idiot, fix the test.', detection.spans, {
  marker: 'la la la',
});
```

Generated declarations live in `native/index.d.ts` after building. JavaScript span offsets are UTF-16 units; Rust spans are UTF-8 bytes. `detect` applies the configured threshold and protection rules. `applyReplacements` applies all supplied spans, validates them, expands subwords, and merges overlaps or whitespace-adjacent spans. Scores must be finite probabilities. One queued or running inference is allowed per detector; concurrent calls fail with `busy`.

Optional typed execution settings select experimental Metal or Core ML backends
in separately built addons. The default remains CPU FP32. See
[backend setup and the 500 ms maximum-latency gate](docs/evaluation.md#inference-latency);
default promotion requires both latency qualification and reviewed quality evidence. Output can exceed
the 64 KiB input limit when validated markers expand the text.

Rust consumers use `craig::{Detector, ModelDetector, ReplacementStrategy, FixedMarker, rewrite}`. Implement either trait to change classification or replacement without changing an agent adapter. The workspace crate is local and has not been published.

The native CLI and Node bridge accept the same JSON request on stdin:

```sh
printf '%s' '{"version":1,"text":"You idiot, fix the test."}' | target/release/craig rewrite
```

Success returns `version`, `status`, `text`, and `replacement_count`. Errors return a content-free `code` and exit nonzero. Prompts are passed through stdin, never process arguments. In applications, avoid putting private text into shell history.

## Privacy boundary

The original exists locally in the composer, mod memory, process stdin, and detector memory. The replacement is passed to downstream submission only after successful processing. The project does not write prompts to files or logs, send them to a remote detector, or provide an original-text fallback.

**A mod is not a complete privacy boundary around Claude Code.** The host owns the composer and process API. This implementation does not prove the absence of host telemetry, crash reporting, terminal capture, or behavior from other plugins. If the host disables, unloads, or skips the mod, its protections do not run. Built-in voice dictation uploads audio before text replacement; use local transcription when the original audio must remain private. See [mods behavior](https://code.claude.com/docs/en/plugins/mods/events) and [voice requirements](https://code.claude.com/docs/en/voice-dictation#requirements).

The classifier can miss offensive wording or classify harmless wording as offensive. Missed text passes unchanged. English social-media training does not establish coverage for other languages or coding prompts. The marker cannot make the original unrecoverable from surrounding context.

Explicit code, quotations, paths, URLs, and technical literals are protected. A detected span touching protected material retains the draft for editing; it is neither silently rewritten nor submitted unchanged. Blank, oversized, malformed, incompatible-model, or failed requests also retain the draft while the mod is active. Input is limited to 64 KiB; inference checks a 30-second deadline between windows, and the adapter limits the bridge to 45 seconds.

## Development

```sh
npm run build:debug
npm run build
npm run verify
npm run eval:local
npm run profile:memory
```

`verify` runs formatting, linting, Rust/property and native tests, TypeScript checks, and Claude plugin validation/tests. Type checking explicitly fetches pinned host declarations when absent. CI builds and tests the native library on macOS arm64 and Linux x64; Claude adapter checks require a local Claude installation.

`profile:memory` uses a synthetic prompt and reports native process resident memory through `/usr/bin/time`. Its Node heap profile covers the launcher only. Debug/release builds, runs, tests, and profiling are also available in `.vscode/tasks.json`.

See [inference latency](docs/evaluation.md#inference-latency) for opt-in CPU/Metal
benchmarks, per-size p99 measurements, and correctness gates.

## Improving the detector

The Rust evaluation CLI measures the current detector, calibrates thresholds, and trains separate classifier heads while keeping the BERT encoder frozen. It uses an offline corpus and an independent local Qwen judge. The training workflow never consumes personal metrics or live prompts. Synthetic bootstrap labels are provisional; candidate weights are not automatically installed or published.

```sh
npm run build:lab
npm run lab:calibrate -- --model .cache/models/mudes-en-base --corpus tests/fixtures/corpus.jsonl --output .cache/calibration.json
npm run lab:train -- --model .cache/models/mudes-en-base --corpus tests/fixtures/corpus.jsonl --policy offensive_spans --output .cache/offensive-candidate
```

`offensive_spans` targets profanity and insulting phrases. `directed_abuse` retains general swearing and targets directed abuse. These are distinct learned heads, not threshold aliases. Library callers select `policy` in `loadDetector`; Rust callers use `ModelDetector::load_policy`. Candidate manifests supply the default threshold. Missing policy heads fail with `policy_unavailable`.

The existing `profiling` build inherits release optimizations and retains debug symbols. The lab reports detector load time, warm inference percentiles, and process peak RSS before starting the separate judge. See [evaluation](docs/evaluation.md#improvement-loop) for judge setup, corpus review, candidate comparison, packaging, and licensing. Model derivatives retain Apache-2.0 notices; application code is licensed under AGPL-3.0-or-later.

See [the architecture decision](docs/adr/0001-local-span-replacement.md) for extension boundaries.

## Fast lexical candidate

The shared loader accepts `loadDetector({detector: 'lexical', modelDir})`, with
`detect`, `rewrite`, and `assess` using the same Rust protection rules. BERT
remains the default. Lexical supports `offensive_spans` on CPU only; unsupported
settings fail without fallback. CRF remains available through the experimental
loader and is not a mod selection.

```sh
npm run build
npm run build:decisions
npm run decisions -- train --corpus tests/fixtures/decision-training-v2.jsonl --output .cache/decision-models-local
export CRAIG_DETECTOR=lexical
export CRAIG_MODEL_DIR="$PWD/.cache/decision-models-local/lexical"
node bridge/cli.mjs doctor
```

Use a new output directory for each training run. Artifacts inherit synthetic
corpus provenance; their labels remain unreviewed. Setup does not promote a
candidate. See [decision evaluation](docs/evaluation.md#decision-detectors) for
quality, parity, screening, qualification and memory commands. Switching the
detector or replacing its artifact requires restarting an existing review worker.

## License, attribution, and forks

This project is free software licensed under the **GNU Affero General Public License v3.0 or later** ([`AGPL-3.0-or-later`](LICENSE)). Anyone may freely run, inspect, and use the software locally at no charge.

Under [LICENSE](LICENSE) (Section 7 additional terms) and [NOTICE](NOTICE):

- **Mandatory attribution**: Every copy, fork, modified version, or redistributed plugin bundle must retain [`LICENSE`](LICENSE) and [`NOTICE`](NOTICE) and credit Adam Williams and `yourmood.ai` with a link to `https://github.com/yourmoodai/craig`.
- **Distinct fork naming**: Any fork or modified version distributed through a Claude plugin marketplace, directory, or package registry must use a distinct plugin `name` and `displayName` and clearly state that it is derived from `yourmoodai/craig`.
- **Upstream improvements**: Any modified version distributed or offered to users must publish its complete Corresponding Source under `AGPL-3.0-or-later` so that improvements can be contributed back upstream.

Model weights are separate Apache-2.0 artifacts; their pinned upstream revisions, third-party conversions, and required notices are recorded in [`models/catalog.json`](models/catalog.json) and generated [`models/NOTICE`](models/NOTICE). Synthetic evaluation fixtures are provided under `CC0-1.0`.

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) for issue acceptance, maintainer response
commitments, development checks, commit standards, and pull request guidance.
