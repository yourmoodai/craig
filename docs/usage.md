# Advanced usage

For installation, see the [README](../README.md). Source builds and developer commands are in [CONTRIBUTING](../CONTRIBUTING.md#development).

## Configuration

Set these environment variables before starting Claude:

| Variable | Meaning |
| --- | --- |
| `CRAIG_DETECTOR` | `bert` by default; opt-in `lexical` requires a decision artifact |
| `CRAIG_MODEL_DIR` | Optional absolute path overriding the automatically prepared model |
| `CRAIG_THRESHOLD` | Override the model threshold; baseline default `0.5`, greater than zero and at most one |
| `CRAIG_POLICY` | `offensive_spans` by default; `directed_abuse` requires a matching candidate head |
| `CRAIG_REPLACEMENT` | Fixed marker, default `ya ya ya`; 1–80 non-control characters |
| `CRAIG_JUDGE_CONFIG` | Optional absolute path to a locally prepared `judge.json`; used only after personal review is enabled |
| `LOG_LEVEL` | `silent` suppresses diagnostic error codes; other values retain them |

The mod rewrites composer submissions. `/craig` opens a local inspection pane with the latest original and replacement; it does not add them to model context. Inspection data is cleared at session start/end. Slash commands and non-composer submissions bypass this adapter.

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
[evaluation workflow](evaluation.md#setup-and-baseline). Missing judge
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

The worker uses local sockets on macOS and Linux and named pipes on Windows. Other agents can reuse
Rust assessment and storage behind their own history and UI adapters.

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
[backend setup and the 500 ms maximum-latency gate](evaluation.md#inference-latency);
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

## License, attribution, and forks

This project is free software licensed under the **GNU Affero General Public License v3.0 or later** ([`AGPL-3.0-or-later`](../LICENSE)). Anyone may freely run, inspect, and use the software locally at no charge.

Under [LICENSE](../LICENSE) (Section 7 additional terms) and [NOTICE](../NOTICE):

- **Mandatory attribution**: Every copy, fork, modified version, or redistributed plugin bundle must retain [`LICENSE`](../LICENSE) and [`NOTICE`](../NOTICE) and credit Adam Williams and `yourmood.ai` with a link to `https://github.com/yourmoodai/craig`.
- **Distinct fork naming**: Any fork or modified version distributed through a Claude plugin marketplace, directory, or package registry must use a distinct plugin `name` and `displayName` and clearly state that it is derived from `yourmoodai/craig`.
- **Upstream improvements**: Any modified version distributed or offered to users must publish its complete Corresponding Source under `AGPL-3.0-or-later` so that improvements can be contributed back upstream.

Model weights are separate Apache-2.0 artifacts; their pinned upstream revisions, third-party conversions, and required notices are recorded in [`models/catalog.json`](../models/catalog.json) and generated [`models/NOTICE`](../models/NOTICE). Synthetic evaluation fixtures are provided under `CC0-1.0`.

## Evaluation and experimental detectors

See [evaluation](evaluation.md) for policy alignment, model provenance, independent judges, training, and profiling. [Decision detectors](evaluation.md#decision-detectors) covers the experimental lexical and CRF alternatives; BERT remains the default. Synthetic labels remain provisional, and model agreement does not establish provider approval. Historical benchmark hashes identify the exact artifacts originally measured; metadata-only edits do not requalify those measurements.
