# 4. Retain local replacement traces only with consent

Date: 2026-10-21

## Status

Accepted for implementation following the explicit choice of opt-in full traces,
local-only storage, seven-day retention, and nonblocking recording failures.
Supersedes only ADR 1's blanket prohibition on prompt persistence. Its offline
inference, deterministic replacement, and no-original-fallback rules remain.

## Context

A trace records what the detector selected, what Savior changed, and what the
adapter subsequently did. Counts alone cannot explain a disputed replacement;
human inspection needs the original context and exact output. Those strings
may contain secrets. Existing personal-review metrics consent permits no text
storage and cannot authorize this new use.

Keeping only metadata preserves the smallest privacy footprint but cannot
support contextual inspection. Excerpts reduce exposure but can omit the
context that distinguishes abuse from quotation. Explicitly consented full
traces support that inspection while keeping content-free metadata the default.

## Decision

Rust owns trace admission, span validation, consent generations, and retention.
The adapter reports its own delivery boundary separately from processing.
Recording reuses actual detector spans; it does not rerun inference to explain
an earlier result. Human annotations distinguish profanity removal, targeted
abuse, and preservation; unresolved judgments remain unresolved.

A private SQLite journal is authoritative. The local inspector queries an
atomically replaced, revision-bound Parquet projection through embedded DuckDB.
The bounded first implementation uses one snapshot rather than daily partitions;
the journal remains available for recovery and regeneration. Annotation changes
invalidate the projection through the same revision contract.

Downloads belong in XDG-style cache storage; retained records belong in private
state storage, isolated by profile. The inspector is a loopback web application
with a per-launch token. No cloud service, remote telemetry, training import, or
Claude history import is authorized by enabling capture.

## Invariants & Abstention

- Full-text recording defaults off and requires independent explicit consent.
  Revocation wins over work admitted under an earlier consent generation.
- Metadata excludes originals, output text, custom marker text, and unkeyed
  prompt hashes. Errors and logs never include prompt content.
- A recording failure warns but does not block successfully filtered content.
  A processing failure still blocks submission; recording cannot create an
  original-text fallback.
- Unsafe paths, invalid spans, and incompatible trace schemas are rejected.
- Retention expires at seven days; storage is bounded within 256 MiB, with
  conservative capacity reservations that may evict earlier. Expiry runs on
  capture and inspection, not while Savior is completely stopped.
- Deletion disables capture and removes managed records, annotations, and
  projections. User exports and external backups are outside that deletion.
- A forwarded record is adapter evidence, never proof of provider receipt or
  approval. Missing model identity or outcome evidence remains unavailable.

## Consequences

Users can inspect and label individual replacements without opening Claude.
Parquet exports preserve a versioned event contract for external analysis.

Opt-in capture creates sensitive local files; owner-only permissions are not
encryption and do not protect against the same user, administrators, backups,
or a compromised machine. Database maintenance and query contention can cause
recording gaps. Full-text traces are not automatically benchmark fixtures.

Operational commands and limitations belong in [local inspection](../inspection.md).
