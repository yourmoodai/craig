# Repository instructions

See [README.md](README.md) for setup and [the architecture decision](docs/adr/0001-local-span-replacement.md) for invariants.

- Keep detection and replacement in Rust; adapters only transport requests and results.
- Never forward original input after a processing failure.
- Never log or persist original prompts. Use synthetic fixtures in tests and profiles.
- Runtime inference must remain offline. Artifact downloads require explicit setup.
- Preserve unmatched source text exactly. Reject ambiguous protected-span edits.
- Use `LOG_LEVEL` for logging; messages and error codes stay lowercase.
- Run `npm run build` and `npm run verify` before concluding implementation work.
- Model changes also require `npm run eval:local` and reference parity verification.
- Never add assistant attribution, signatures, or contributor footers.

## Evaluation language

- Use **measured policy alignment** for provider-policy evaluations.
- Separate replacement correctness, agreement with reference labels, and cross-model agreement in documentation, UI, reports, and result summaries.
- Model agreement never establishes provider approval, enforcement guarantees, reviewed labels, or judge calibration. Do not describe evaluation as certification.
- Keep provisional labels and unresolved judgments visible; report implementation checks separately from model-quality outcomes.
