# Craig

<img src="docs/assets/craig.png" width="180" alt="Craig wearing a blue hat">

Craig replaces detected offensive language in your Claude Code prompts with `ya ya ya`, using a filter that runs on your computer.

## Inspect the receipts locally

The standalone inspector shows exact replacement diffs and model provenance,
with separate human judgments for profanity, targeted abuse, and preservation.
Run `npm run inspect` after building from source. Full-text recording is opt-in;
records stay private and local, with seven-day retention and a 256 MiB ceiling.

| New boundary | Local verification |
| --- | --- |
| Signed bootstrap | Signature rejection, interrupted download, concurrent setup, crash recovery, and offline reuse |
| Real compiled runtime | macOS ARM64 binaries and native addon exercised with an empty PATH |
| Trace store | Six checks covering consent, exact edits, retention, annotations, recovery, and Parquet |
| Browser inspector | 12 checks passed: consent, Unicode edits, annotations, literal markup, access controls, export, deletion, and nonblocking recording failure |

The public GitHub download matrix and Windows/Linux runtime checks remain CI
release gates, not locally established claims. Timing screens vary substantially
with machine load and do not qualify latency.

The final five-sample short-input screen measured 24.3–32.7 ms without capture,
24.8–31.1 ms with metadata, and 25.2–26.1 ms with full traces. Cold/warm Parquet
export took 31.1/13.3 ms for ten synthetic records; peak parent-process RSS was
about 480 MiB including the detector. A separate busy-machine run reached
432 ms, so these figures are observations, not overhead guarantees.

[Browser evidence](benchmarks/results/inspection-e2e-2026-10-21.json) ·
[Final timing screen](benchmarks/results/inspection-final-2026-10-21.json) ·
[Busy-machine screen](benchmarks/results/inspection-2026-10-21.json) ·
[Inspection guide and reproduction commands](docs/inspection.md).

<!-- benchmark-results:start -->
## Battle-tested, with receipts

Craig’s replacement engine is checked for exact preservation outside selected spans. Detection is measured separately against public reference labels.

| Check | Observed result |
| --- | --- |
| Known-span replacement and abstention | 42/42 checks passed |
| Rust and Node CLI end-to-end checks | 36/36 checks passed |
| Installed Claude Code delivery and recovery | 15/15 requested checks passed |
| Scunthorpe flag agreement | 74.5% on 188/200 sampled cases; F1 0.739; 12 abstentions |
| HateCheck hate-label agreement | 62.2% on 193/200 sampled cases; F1 0.757; 7 abstentions |
| Unmatched text: `fixed_phrase` | 381/381 successful outputs preserved; 19 abstentions |
| Unmatched text: `custom_phrase` | 381/381 successful outputs preserved; 19 abstentions |
| Unmatched text: `omit` | 381/381 successful outputs preserved; 19 abstentions |

Measured 2026-10-21 UTC on Apple M5 (darwin, arm64), CPU FP32, pinned `mudes/en-base`.
The external screen samples English cases by category and label; it is not a population accuracy estimate. HateCheck labels hate speech, which differs from Craig’s offensive-span policy.
Reviewed external replacement spans are unavailable. These classification figures do not establish successful removal of targeted abuse.

| Resident rewrite fixture | Native maximum | Node-API maximum | Failed combinations |
| --- | ---: | ---: | ---: |
| short | 39.88 ms | 37.62 ms | 0/4 |
| medium | 174.36 ms | 183.93 ms | 0/4 |
| long | 862.72 ms | 888.88 ms | 0/4 |
| maximum | 16.39 s | 17.03 s | 0/4 |
| dense | 23.09 s | 23.69 s | 0/4 |
| unicode | 27.72 s | 28.61 s | 0/4 |
| protected | unavailable | unavailable | 4/4 |
| token_dense | unavailable | unavailable | 4/4 |
| window_boundary | 1.78 s | 1.83 s | 0/4 |
| tokens_509 | 607.60 ms | 581.08 ms | 0/4 |
| tokens_510 | 582.60 ms | 600.22 ms | 0/4 |
| tokens_511 | 757.13 ms | 758.90 ms | 0/4 |
| tokens_892 | 1.17 s | 1.20 s | 0/4 |
| tokens_893 | 1.35 s | 1.34 s | 0/4 |
| punctuation_dense | unavailable | unavailable | 4/4 |

Performance coverage: 48/60 measurements completed successfully; 12 failed. Maxima above combine strategies only within the same fixture; failed timings are not success samples.
Recorded configuration: 1 run(s), 1 measured sample(s) and 0 warmup(s) per fixture, strategy, and boundary; 45 s worker deadline.
This is a screen, not latency qualification. The all-input maximum-below-500-ms screen did not pass. Cold load, process startup, per-fixture timing, and peak RSS are recorded separately in the evidence.
Latency runs exclude stage profiling. Matched per-fixture strategy comparisons and explicit failure causes are retained in the snapshot.
Failure classifications: 12 `processing_timeout`.
Machine context: 10 logical CPUs; start/end 1-, 5-, and 15-minute load averages 7.09 / 7.84 / 7.25 → 6.82 / 7.47 / 7.38.
Per-measurement 1-minute load ranged from 6.29 to 8.94. Background work was not controlled; load alone does not establish the cause of a timeout.

[Evidence snapshot](benchmarks/results/latest.json) · [Reproduce the checks](docs/benchmarking.md)
<!-- benchmark-results:end -->

## Installation

**The first release is not published yet.** The commands below will work once it is available. Developers can [build from source](CONTRIBUTING.md#development).

You need Claude Code installed and a Mac with Apple silicon, or Windows/Linux on an Intel or AMD 64-bit computer.

1. Open **Terminal** on macOS/Linux, or **PowerShell** on Windows.
2. Paste these commands, one at a time:

   ```sh
   claude plugin marketplace add https://github.com/yourmoodai/craig/releases/latest/download/marketplace.json
   claude plugin install craig@yourmood.ai
   ```

3. Open Claude Code, enable Craig if prompted, and type `/craig` to check that it is ready.

The first activation automatically downloads the matching platform runtime and
the filter’s model (about 431 MB). Craig verifies a signed manifest and artifact
hashes before running downloaded code. Wait until Craig is ready before submitting
a prompt. If setup fails, open `/craig` to retry. No additional development tools
are needed; the plugin includes its bootstrap Node runtime.

[Get help](https://github.com/yourmoodai/craig/issues) · [Advanced usage](docs/usage.md)
