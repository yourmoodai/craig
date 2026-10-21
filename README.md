# Craig

<img src="docs/assets/craig.png" width="180" alt="Craig wearing a blue hat">

Craig replaces detected offensive language in your Claude Code prompts with `ya ya ya`, using a filter that runs on your computer.

<!-- benchmark-results:start -->
## Battle-tested, with receipts

Craig’s replacement engine is checked for exact preservation outside selected spans. Detection is measured separately against public reference labels.

| Check | Observed result |
| --- | --- |
| Known-span replacement and abstention | 42/42 checks passed |
| Rust and Node CLI end-to-end checks | 36/36 checks passed |
| Installed Claude Code submissions | 3/3 requested checks passed |
| Scunthorpe flag agreement | 74.5% on 188/200 sampled cases; F1 0.739; 12 abstentions |
| HateCheck hate-label agreement | 62.2% on 193/200 sampled cases; F1 0.757; 7 abstentions |
| Unmatched text: `fixed_phrase` | 381/381 successful outputs preserved; 19 abstentions |
| Unmatched text: `custom_phrase` | 381/381 successful outputs preserved; 19 abstentions |
| Unmatched text: `omit` | 381/381 successful outputs preserved; 19 abstentions |

Measured 2026-10-21 UTC on Apple M5 (darwin, arm64), CPU FP32, pinned `mudes/en-base`.
The external screen samples English cases by category and label; it is not a population accuracy estimate. HateCheck labels hate speech, which differs from Craig’s offensive-span policy.
Reviewed external replacement spans are unavailable. These classification figures do not establish successful removal of targeted abuse.

| Resident rewrite screen | p50 | p95 | p99 | Maximum | Samples |
| --- | ---: | ---: | ---: | ---: | ---: |
| native_fixed_phrase | 702.64 ms | 21.51 s | 21.51 s | 21.51 s | 11 |
| native_omit | 863.67 ms | 21.29 s | 21.29 s | 21.29 s | 11 |
| node_api_fixed_phrase | 1.06 s | 28.20 s | 28.20 s | 28.20 s | 13 |
| node_api_omit | 940.51 ms | 28.52 s | 28.52 s | 28.52 s | 12 |

Performance coverage: 47/60 measurements completed; 13 failed. Distributions combine the recorded fixture workload through 64 KiB.
Recorded configuration: 1 run(s), 1 measured sample(s) and 0 warmup(s) per fixture, strategy, and boundary; 45 s worker deadline.
This is a screen, not latency qualification. The all-input maximum-below-500-ms screen did not pass. Cold load, process startup, per-fixture timing, and peak RSS are recorded separately in the evidence.

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

The first launch automatically downloads the filter’s model (about 431 MB). This may take a few minutes. Wait until Craig is ready before submitting a prompt. If the download fails, open `/craig` to retry. No additional development tools are needed.

[Get help](https://github.com/yourmoodai/craig/issues) · [Advanced usage](docs/usage.md)
