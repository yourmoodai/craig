# 1. Share local replacement across interception layers

Date: 2026-10-12

## Status

Accepted for the Rust library, mod delivery, and architectural boundaries. The privileged proxy
is a researched future direction; its implementation and enforcement claims
remain subject to the verification gates below.

## Context

Users need control over what leaves their machine. The original wording must be available to a local detector without being submitted to a remote classifier. Agent adapters differ in their execution environments: Claude mods expose a process API but cannot directly load a native Node addon.

A generative rewriter could paraphrase whole prompts but can alter instructions. A word list offers predictable matching but misses contextual phrases. A token classifier predicts source spans, allowing deterministic replacement while retaining surrounding instructions.

Text replacement and control over delivery are separate responsibilities.
Replacement determines which source spans become `ya ya ya`, or another
configured marker. Delivery control determines whether the original can leave
the machine through a path that bypasses replacement.

The Claude mod integrates replacement into the composer workflow and provides
local inspection. It depends on the host running the handler. A future network
gateway would instead own the outbound request boundary, independently of the
mod and Claude configuration. This provides an architectural option if provider
policies or host extension capabilities become more restrictive.

An explicit HTTP proxy is useful for inspecting cooperative applications, but
proxy settings do not compel every connection to use it. Passive packet capture
can verify traffic and decrypt it when session secrets are available; it does
not prevent the captured bytes from reaching their destination. A transparent
proxy combined with independent network enforcement can impose a delivery
boundary. TLS inspection then needs separate client-facing and upstream TLS
sessions, not modification of ciphertext in transit.

The workshop already provides a passive Rust interception engine, host/FFI
boundaries, and a macOS transparent-proxy shell. Its current transport forwards
encrypted bytes unchanged and decrypts a copy using supplied TLS secrets.
Those observation semantics must not become the future enforcement policy.

## Decision

### Release distribution and initialization

Distribute a versioned GitHub Release ZIP through an archive marketplace source,
pinned by SHA-256. Include precompiled Node runtimes, Rust CLI, Node-API addon,
review worker, JavaScript dependencies, and notices for macOS arm64, Linux x64,
and Windows x64. The archive must be below 210,000,000 bytes (210 MB). Models
are separate pinned release assets because the existing detector compresses to
approximately 400 MB; packaging does not authorize a detector change.

Installing the release authorizes first-activation model setup. A Rust setup
command owns resumable downloads, file locks, digest verification, and atomic
promotion into a per-user cache. Concurrent sessions share a download; a crashed
process releases its lock. Explicit model overrides are validated, never
silently replaced. Runtime inference stays offline. The mod uses bundled
executables by absolute path and withholds composer submissions until setup and
a native doctor check succeed. Failure retains the draft; `/craig` exposes retry.

Windows workers use user-restricted named pipes and protected metrics-directory
ACLs; Unix workers retain private sockets. Both transports preserve request-size
limits, cancellation, singleton ownership, and idle shutdown. Metrics and history
remain opt-in; optional judges are not part of baseline activation.

Build and smoke-test each platform in CI, then verify the assembled ZIP on clean
profiles. Actual installed-composer tests are required for production publication;
missing host coverage cannot be substituted by a mocked handler or CLI test.
Tag releases must match package versions and be reachable from main. Publish
immutable assets before updating the marketplace pointer; retain prior releases
for rollback. No release has qualified until these gates pass.

Bedrock Haiku 4.5 replaces the exhausted Kimi reviewer and unactivated Copilot
transport. It is a metered CI-only reviewer, separate from local inference. GitHub
OIDC grants short-lived access to one US inference profile through a dedicated
environment restricted to main and develop. Live review remains disabled until
account access and model qualification succeed. Trusted base code reviews PR
diffs as data; no PR executable receives review secrets. Required tests must
pass for the exact head before review. A single editable comment carries the
50–100 word overview, verdict, coverage, and cost. Complete qualified assessments
submit a formal approval when clean or request changes for validated findings.
Infrastructure failures fail CI without a defect review. There is no size-based
or code-owner bypass; branch protection remains unchanged.

Review packing owns contiguous change blocks, retaining stable evidence IDs and
old/new coordinates through regrouping and subdivision. Dynamic token ceilings
grow with reviewable units; full rendered prompts must still fit bounded input
and output budgets. Supporting context is disclosed separately from primary
coverage. Truncation or saturated findings causes smaller assessments; no parent
counts as complete until all children finish. Fixed 48 KiB packing remains the
baseline until the same bounded fixtures demonstrate preserved labels and lower
adaptive cost. This trades extra planning and token-count calls for measurable
coverage and spending control; semantic correctness is not guaranteed by packing.

The $2 pilot shares an atomic ledger across qualification, comparison, assessment,
verification, synthesis, and resumed attempts. At most $0.40 funds qualification
and comparison. Reserve worst-case counted inference cost before calling Bedrock;
retain uncertain charges after interruptions. CI persists a reservation before
inference, restores reconciled ledgers, and abstains if spending history is missing.
Pricing assumptions, thresholds, and run commands belong in the
[reviewer guide](../../tools/adversarial_reviewer/README.md). Explicit prompt and
result caching remain off until measured evidence justifies their complexity.
Stale results are discarded.
Non-executable PNG/JPEG/WebP files under `docs/assets` are disclosed exclusions;
other binary omissions prevent approval. This can block merging when review
infrastructure fails, preserving the distinction between a clean review and an
unavailable one. Neither verdict replaces required technical checks.

The tradeoff is a larger bundled runtime and a first-use network dependency.
Subsequent use needs neither developer tooling nor a network connection. Release
artifacts are generated by `scripts/release/package.py`; the release workflow
owns platform qualification and publication rather than adding a second ADR.

### Delivery order and shared core

Deliver the Claude mod first. Keep detection, span normalization, protected-text
checks, replacement strategies, and content-free errors in the existing Rust
core. Use pinned `mudes/en-base` artifacts with Candle and Hugging Face tokenizers
for CPU inference. `Detector` and `ReplacementStrategy` support alternative
implementations; `RewriteResult` carries the replacement result. `FixedMarker`
defaults to `ya ya ya` per merged span.

```text
composer text → mod / Node-API adapter ───┐
                                        ├→ Rust replacement core → adapter delivery
HTTP text fields → future HTTP adapter ─┘
```

Node-API exports the library with generated TypeScript declarations. The mod
calls a Node bridge through `process.run`, supplying JSON on stdin; the bridge
returns sanitized text only. The native CLI exposes the same versioned transport
for other adapters. A future HTTP adapter extracts supported text fields, calls Rust
directly, and reconstructs the request. That adapter owns JSON structure,
compression, framing, and protocol handling. TLS, routing, privileges, and
platform lifecycle remain outside the replacement core.

Transport limits belong to adapters; the core retains its text-level limits.
A future adapter must reject unsupported or oversized requests without sending
their original bodies. It must preserve the semantics of supported non-text
fields and map source offsets within each decoded text field, rather than
applying offsets to serialized HTTP or JSON bytes.

Preparing for the deferred proxy requires no public API change, new proxy
dependency, speculative platform stub, or OS configuration change. The existing mod does not need the
future proxy to operate. The future proxy must not need the mod to enforce its
policy.

### Model evaluation and improvement

Classifier quality is empirical: exact replacement behavior does not establish
that every unwanted phrase is detected. Maintain an offline evaluation loop in
a separate Rust development executable. Keep model training and judging out of
the runtime library's dependency graph; the optional `training` feature exposes
detached encoder activations without network clients or optimizers.

Evaluate a versioned, licensed corpus under `offensive_spans` and
`directed_abuse` policies. Keep scenario families in one split and reserve test
examples from calibration and training. Synthetic labels bootstrap the workflow;
judge proposals require review before training. Reports distinguish deterministic
invariants, reference-label agreement, and the judge's categorical opinions.

Use the independently trained `Qwen3-4B-Instruct-2507` as the initial judge,
running a pinned GGUF conversion in an owned, offline `llama-server` process on
loopback. Pin the rubric and inference settings. Start judging after detector
profiling and record representative samples separately from failure-enriched
samples. Missing, malformed, refused, or uncertain judgments provide no passing
evidence. Judge calibration requires reviewed examples; synthetic scores do not
constitute release approval or formal verification.

Calibrate thresholds before changing weights. Fine-tune separate binary
classifier heads using Candle AdamW and a frozen encoder. Preserve tokenizer,
windowing, span normalization, and protected-text semantics. Normalize repeated
window contributions during training. Select checkpoints and thresholds using
validation examples only, then compare against the locked test corpus.

Candidate manifests bind a policy and threshold to the pinned base revision and
classifier hash. Runtime loading verifies the original base hashes and candidate
head. These checks establish artifact integrity, not publisher authenticity.
Export/reload parity and an unchanged encoder are required evidence. Candidate
packaging includes the model license, modification notices, provenance, and
evaluation report; it never changes the installed default or publishes a model.

The baseline weights are published under Apache-2.0, allowing modification and
redistribution subject to notices and license requirements. Data rights are
separate. Keep newly supplied corpus provenance and licensing explicit. Full
encoder fine-tuning, private training-data collection, and automatic promotion are deferred.
Reproduction and acceptance criteria belong in [evaluation](../evaluation.md#improvement-loop).

### Inference performance experiments

Keep CPU FP32 batch-one execution as the default while benchmarking opt-in
Accelerate, Metal FP32/FP16, and macOS Core ML execution. Rust owns device
selection, tokenization, masked window batching, and source-offset reconstruction.
A small Objective-C bridge transports Core ML tensors and retains compiled
assets until model destruction; Python is export-only. Flexible exports are
restricted to CPU/GPU compute units after observed crashes/stalls in the
CPU/Neural Engine configurations. Fixed-shape experiments remain available;
reconsider this restriction only after stable full-request runtime checks.
Predetermined padded shapes allow optimized Core ML execution without dropping
windows. Export metadata binds package hashes to the original model and head.
Unavailable backends fail without fallback. Optional typed Node-API execution
settings expose experiments through separately built addons.

The revised target is maximum observed resident rewrite latency **below 500 ms**
through the Node-API boundary, across supported input sizes through 64 KiB on
the measured M5. It replaces the 600 ms target, which replaced the earlier 25 ms p99 target; historical reports
retain their meaning. Startup and optional assessment/judging remain separate.
A timeout, truncated prompt, skipped window, or answer cache cannot qualify.
Protected-span abstention remains explicit. Marker expansion uses a checked
output bound derived from validated spans and markers; the input limit must
not reject otherwise valid expanded output.

Unchanged-weight execution must preserve offsets, replacement outcomes, and
reference probabilities within `0.0001`, including reduced precision. Do not
relax parity to promote a faster backend. Qualification requires all native and
FFI samples below 500 ms in three 10,000-sample runs per fixture, with at least
20 warmups, full coverage, and no unexpected errors. This is measured evidence,
not an operating-system deadline guarantee. Model replacement requires separate
reviewed held-out quality evidence.

Acceleration adds platform build dependencies, export/setup work, and device
memory costs. No candidate becomes the default while quality evidence is missing
or any latency bucket fails. Benchmark commands, limitations, and results belong
in [evaluation](../evaluation.md#inference-latency).

### Fast detector promotion

Use lexical detection as the first fast replacement candidate. It matches a
versioned vocabulary and bounded punctuation-separated ASCII spellings, returning
original source offsets to the existing protection and replacement pipeline.
The first screen measured an 8.43 ms Node-API maximum versus CRF's 50.38 ms,
with identical independent quality outcomes (8/12). BERT preserves learned
context but failed the full-input latency screen; CRF remains a challenger
until its added computation demonstrates a quality benefit.

`RuntimeDetector` owns explicit BERT/lexical selection for the library, CLI,
Node-API and personal assessment. Existing callers default to BERT. The mod can
opt into lexical artifacts with `CRAIG_DETECTOR=lexical`. Unsupported
policies, execution settings and artifacts fail without fallback. A lexical
failure never invokes BERT or forwards the original. Training and independent
judging remain offline development operations, outside submission inference.

Promotion requires the 500 ms qualification protocol above, passing rewrite
regressions, reviewed labels, no additional edits to clean evaluation cases,
correct protected abstentions, and normalized removal precision and recall at
least matching BERT on the same held-out corpus. A different detector does not
need BERT probability equality; unchanged-weight backends still do. Inspected
held-out failures become regressions and cannot remain untouched evidence after
tuning. Preserve original reports and dataset provenance. No default changes
while any gate is missing or failing.

Lexical inference reduces compute and model size but cannot establish broad
semantic coverage. Scores are detector confidence, not calibrated severity.
Bounded spelling recognition may miss unsupported obfuscations. Protection
continues to decide whether any detected span can be rewritten. Full mod
startup and optional assessment remain separately measured costs.

### Measured policy alignment and personal review

Extend the shared Rust boundary with `craig-review`: source-aware language
assessment, versioned policy interpretation, bounded judging, and metric storage.
The native detector exports `assess` through Node-API. The mod transports inputs
and renders results; it does not implement classification, retention, or date
filtering. The offline lab and optional worker share one local judge runner.
Its network client is behind the `judge` feature and talks only to its owned
loopback server; the native detector remains free of network dependencies.

The policy record binds the model-directed-abuse clause to its source URLs,
effective date, retrieval timestamps, and hashes. Fetching updated pages is an
explicit development action and does not silently change the interpretation.
Contextual judgment is a project-defined estimate. There is no published
numeric removal threshold; missing reviewed labels or calibrated
judge evidence requires an inconclusive result. A reference replacement mismatch
remains a failure even when the judge approves it.

Live review and history access require separate, profile-scoped consent. Live
review assesses every processed composer prompt, sampling approximately 10% for
the judge with a 100-sample daily cap. History scans also sample approximately
10%, but are explicitly started and bounded to one through five days, default
three. The SDK reconstructs local session chains across the active profile;
a narrow, versioned Rust reader supplies per-message timestamps omitted by the
SDK. Unknown versions and incomplete records reduce reported coverage. Stored
user messages are not authenticated originals, so history and live summaries
must remain distinct.

Use a separate local worker over an owner-only Unix socket because the mod host
offers bounded `process.run` calls but no detached process lifecycle API. A
Node launcher starts the worker; Rust owns its four-job queue, one active job,
consent generations, cancellation, sampling, and SQLite store. A worker with
different model or judge configuration is rejected until explicitly restarted.
Other platforms need their own authenticated local transport before support is
advertised. This transport does not implement the deferred network proxy.

Persist only typed metrics, categorical judgments, and provenance hashes for
30 days. HMAC identifiers permit deduplication without storing session IDs or
prompt IDs. Live context contains at most four preceding replacements; history
context contains at most four preceding eligible stored user messages. Both
are bounded to 16 KiB and remain in memory. Neither provides a complete
conversation or establishes the user's purpose. Consent changes invalidate
pending writes; cancellation releases queued text and kills active judge
processes. An in-flight detector can finish locally before releasing its input.
No private text is written to training corpora, logs, or metric exports.

Metric failures leave successful replacement intact. Failed replacement still
retains the draft. Queue saturation, unavailable judges, unknown history formats,
and uncertain judgments must never count as clean observations. Independent
tests cover Rust invariants, native measurements, SDK history reconstruction,
mod lifecycle, and installed-host submission; none establishes absence of host
telemetry. Reproduction and measured limitations belong in
[evaluation](../evaluation.md#policy-and-personal-review).

### Independent judge comparison

Use measured policy alignment as the evaluation claim. Keep replacement
correctness, agreement with reference labels, and cross-model agreement separate.
Provider approval and enforcement outcomes are outside those measurements.

The offline comparison prepares detector outputs once and runs pinned Qwen and
Gemma 4 12B presets sequentially. Both see identical source text, replacements,
context, marker, and policy interpretation. Expected answers, detector identity,
and competing verdicts are excluded. Each process uses its model's embedded
chat template; artifact, runner, rubric, and generation settings are recorded.
The configured Qwen runner remains supported; Gemma uses the repository-local
llama.cpp v0.6.0 distribution, build b11429.

An agreement is evidence about these judges, not ground truth. Uncertain or
missing answers are unresolved even when both judges produce them. Disagreement
has no automatic tie-breaker and cannot alter reference labels or promote a
model. Comparison is restricted to the offline corpus; live/history defaults
remain single-judge. Setup may download pinned artifacts explicitly; inference
remains offline. New policy reports and metric exports use schema version 2
with `policy_alignment_status`; historical reports remain unchanged.

### Future macOS enforcement

Use a signed Network Extension system extension based on
`NETransparentProxyProvider` to acquire outbound flows. A shared Rust gateway
owns destination classification, TLS termination, HTTP adaptation, and calls to
the replacement core. A privileged installer owns activation, trust setup,
service installation, and removal. Root permissions do not replace Apple
signing, provisioning, notarization, or system-extension approval requirements.

Include a persistent PF firewall guard in the future feasibility work. Its
intended invariant is that external application traffic can leave only through
gateway-owned sockets, including when the gateway or Network Extension dies.
Use a dedicated gateway identity rather than exempting every root process.
Identified unrelated traffic is relayed unchanged through the gateway; it must
not require a broad direct-egress exemption that defeats crash protection.

PF integration is a candidate mechanism, not an established guarantee. Apple
does not consider PF a supported product API. Rule ordering, existing states,
identity matching, boot activation, network-control exceptions, and interactions
with system services need validation for each supported macOS release.
Environments that have not passed the enforcement verification gates must not activate or advertise protected operation.
A watchdog restart alone is insufficient because it permits a possible gap.

The gateway uses a locally trusted CA to present client-facing certificates and
independently validates upstream certificates and hostnames. CA private material
must be inaccessible to ordinary clients, and privileged control channels must
authenticate their peers. No shared CA private key ships with the product.
Certificate rejection or pinning results in a closed connection, never an
uninspected retry. Client compatibility must be measured with the actual binary.

### Traffic policy

| Classified traffic | Intended action |
| --- | --- |
| Supported Anthropic request | Inspect, replace supported text, validate, then forward |
| Identified telemetry request | Suppress without forwarding its payload |
| Identified unrelated destination | Relay unchanged without payload inspection |
| Ambiguous destination or unsupported protected protocol | Block |
| Inspection, model, certificate, or gateway failure | Withhold the protected request |

Destination classification must account for hostname evidence, destination
addresses, and conflicts between them. Shared addresses alone cannot reliably
identify a service. Third-party telemetry collectors need explicit policy
coverage even when their names do not contain `anthropic`. Telemetry sharing an
API domain requires request-level classification; a domain blocklist is not
sufficient.

Encrypted hostnames, opaque tunnels, and unsupported QUIC flows may prevent
classification. The selected default is to block ambiguous flows, accepting
compatibility costs. Known unrelated domains are not decrypted to search for
arbitrary hidden downstream destinations. The guarantee consequently excludes
deliberate relaying through a permitted unrelated service.

### Platform contracts

Keep the backend contract independent of CPU architecture and OS object types.
It must describe captured flow identity, original destination, transport,
direction, optional verified process identity, and the provenance of hostname
information. Its responsibilities include flow ownership, authenticated gateway
handoff, backpressure, connection closure, independent egress enforcement,
atomic policy activation, lifecycle, and health reporting.

| Platform | Future native implementation boundary |
| --- | --- |
| macOS | Swift Network Extension and privileged lifecycle shell; Rust gateway; independently validated PF guard |
| Linux | Netfilter/nftables interception and egress policy; original-destination and socket-mark handling |
| Windows | WFP connection redirection and filtering; privileged service and required signed callout driver |

Target arm64 and amd64 on all three platforms. Shared Rust builds and contract
tests establish portability of the core, not enforcement support. Each backend
must independently pass the same bypass and failure suite on real supported
systems before it can report protection. Linux and Windows implementations are
deferred alongside the proxy.

### Workshop extraction

Authorization is recorded to extract selected workshop code and relicense that
code under AGPL-3.0-or-later, leaving the workshop intact. Reuse applicable activation,
transport, flow vocabulary, FFI ownership, and test patterns. Preserve applicable
third-party copyright and license notices; authorization for workshop code does
not change third-party licenses.

Replace passive forwarding, permissive abstention, raw flow logging, and
key-log-dependent operation in the enforcement path. The passive engine may
inform synthetic verification tooling but cannot serve as the rewriting gate.
Use `craig` naming for extracted crates, commands, bundle identifiers,
services, logs, and documentation. Extraction is deferred; this ADR does not
move or rename workshop files.

## Invariants & Abstention

- Replacement stays local and independent of agent and networking APIs. No
  remote classifier, original-content logging, or original-content error output.
- Inference has no runtime network client. Artifact downloads are explicit and
  hash checked; model loading verifies every artifact again.
- Preserve bytes outside selected spans. Rust offsets use UTF-8 bytes and
  JavaScript offsets use UTF-16 units. Protected-text conflicts remain errors;
  a network adapter must not silently relax core protection rules.
- Reject malformed input, invalid offsets, incompatible models, and processing
  failures. Bound core text input to 64 KiB, allow one queued or running inference
  per loaded Node detector, and use overlapping windows with a processing deadline.
- Keep original/replacement inspection in session memory, outside model context.
  The mod does not control host telemetry, unloaded or other plugins, voice
  uploads, or unrecognized offensive wording.
- The mod retains a draft on processing failure. The future proxy withholds the
  request. Neither uses original text as an error fallback.
- A protected HTTP request releases no application payload upstream until its
  complete supported request representation has passed processing. Retries,
  redirects, and repeated conversation history must cross the same gate.
- Enforcement failure must not become direct egress. Ambiguous traffic is
  blocked, and known unrelated traffic is relayed without rewriting.
- Activation is refused when required enforcement, trust, model, or health
  checks fail. Disabling or uninstalling protection is an explicit privileged
  operation, not an automatic recovery action.
- Claims distinguish enforced processing from detector accuracy. False negatives
  remain possible; this design does not guarantee removal of every unwanted
  phrase or prevent inference from retained context.
- The boundary assumes an uncompromised OS and gateway. It does not protect
  against a privileged administrator disabling enforcement, retroactively erase
  prior disclosures, or control the remote provider's response policies.

### Verification gates for the future proxy

Use synthetic canaries and an independently controlled upstream receiver.
Packet capture establishes routing evidence; upstream plaintext observations
establish payload evidence. Never infer successful rewriting merely from the
absence of readable text in encrypted captures.

| Gate | Required evidence |
| --- | --- |
| TLS compatibility | Actual client accepts the local CA; certificate rejection and pinning never cause pass-through; upstream validation remains enabled |
| Request fidelity | HTTP/1.1 and HTTP/2, compressed bodies, framing boundaries, supported text fields, repeated history, retries, and redirects preserve required semantics |
| Streaming | Responses stream correctly; unsupported request streaming cannot release unprocessed fragments |
| Telemetry | Known collectors and telemetry on shared API domains receive no suppressed payloads |
| Bypass resistance | IPv4/IPv6, direct IPs, nonstandard ports, QUIC, encrypted hostnames, DNS paths, tunnels, and loopback relays cannot evade the declared policy |
| Failure containment | Gateway and extension termination, overload, malformed requests, model failures, pre-existing connections, and restart races release no protected original payload |
| Lifecycle | Startup, reboot, sleep/wake, interface changes, upgrades, and firewall conflicts preserve the enforcement invariant |
| Compatibility | Identified unrelated traffic is unchanged; exceptions are explicit and do not create an alternate route for protected requests |
| Portability | Each advertised OS/architecture combination passes native enforcement tests, not only compilation |

The unresolved questions are whether the actual client accepts interception,
whether the proposed macOS guard survives every required lifecycle transition,
and which request schemas and telemetry routes must be supported. The future
implementation must answer them before claiming the guarantee. No installation
or live network interception has been performed as part of this ADR.

## Consequences

The mod can deliver the current workflow while the Rust library remains usable
at different interception layers. A future gateway can enforce delivery without
relying on Claude's extension behavior or proxy configuration.

CPU inference loads a roughly 431 MB artifact for each rewrite bridge invocation.
Optional history review retains its own detector in a worker until idle shutdown;
sampled judging adds separate model memory. Retention cleanup requires opening
the worker, and exported metric copies remain user-managed. English social-media
training does not establish accuracy for coding instructions or other languages.
Literal protection can require users to edit a retained draft. Host-level
failures remain outside the mod's enforcement boundary.

Privileged enforcement introduces certificate custody, signing, installation,
OS-specific testing, and recovery obligations. Blocking ambiguous connections
can interrupt unrelated applications. Gateway failure may deliberately interrupt
network access rather than reopen a bypass. The PF approach adds compatibility
risk because it is not a supported product API. These costs belong to the future
proxy effort and do not expand the mod's current delivery scope.

### Research evidence

- [Apple transparent-proxy API](https://developer.apple.com/documentation/networkextension/netransparentproxyprovider): declining a flow permits direct communication; an inspection error must explicitly close an owned flow.
- [Mitmproxy macOS local capture](https://www.mitmproxy.org/posts/local-capture/macos/) and [redirector source](https://github.com/mitmproxy/mitmproxy_rs/blob/main/mitmproxy-macos/redirector/network-extension/TransparentProxyProvider.swift): demonstrated application-transparent capture through Network Extension, not evidence of crash-proof enforcement.
- [Apple provider deployment](https://developer.apple.com/documentation/technotes/tn3134-network-extension-provider-deployment): system-extension packaging and deployment constraints. System extensions operate independently of the logged-in user.
- [Apple PF guidance](https://developer.apple.com/documentation/technotes/tn3165-packet-filter-is-not-api): PF is an administrator facility rather than a supported product API, with potential conflicts involving other rules and system services.
- [Apple content-filter source](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/net/content_filter.c): the inspected `cfil_ctl_disconnect` path passes queued data while detaching a filter. This motivates independent enforcement; source inspection alone does not establish behavior on every shipped OS.
- [Squid selective TLS inspection](https://www.squid-cache.org/Doc/config/ssl_bump/): separate inspect, tunnel, and terminate decisions. Its default tunnel behavior is not the policy selected here.
- [Wireshark TLS analysis](https://wiki.wireshark.org/TLS/): passive decryption requires appropriate secrets; useful for verification rather than delivery enforcement.
- [Apache Traffic Server transparent forwarding](https://docs.trafficserver.apache.org/en/latest/admin-guide/configuration/transparent-forward-proxying.en.html): precedent for separating transparent traffic redirection from proxy request handling.
- [Linux transparent proxy support](https://docs.kernel.org/networking/tproxy.html): Netfilter interception, routing, and transparent-socket requirements for a future Linux backend.
- [Windows connection redirection](https://learn.microsoft.com/en-us/windows-hardware/drivers/network/using-bind-or-connect-redirection) and [WFP operation](https://learn.microsoft.com/en-us/windows/win32/fwp/basic-operation): native connection-redirection and filtering mechanisms, including persistent and boot-time filter lifecycles.
