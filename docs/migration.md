# Moving to Savior

Savior is the final name of this Claude Code mod. Its local replacement behavior
and default `ya ya ya` phrase are unchanged.

## Installation and configuration

Disable and uninstall the old `craig@yourmood.ai` plugin before installing
`savior@yourmood.ai`. Do not enable both: a prompt must pass through one instance
of the mod. Follow the [installation instructions](../README.md#installation)
once an installable release is available. For source development, follow
[Contributing](../CONTRIBUTING.md#development).

Use `/savior` in Claude Code and `savior` for the CLI. Change shell configuration,
editor tasks, and automation from `CRAIG_*` to `SAVIOR_*`; suffixes and values
retain their meanings. Old commands and variables are not compatibility aliases.

## Private state

Savior uses its own `savior` cache and state directories. Existing `craig`
directories, trace records, metrics, and consent settings are left untouched.
There is no automatic import or deletion. A fresh Savior installation starts
with recording disabled; explicitly enable any recording you want through the
[inspection controls](inspection.md). Cached models can be configured with
`SAVIOR_MODEL_DIR` without copying private history.

Export any records you need with your existing installation before uninstalling
it. Exports contain private data; keep them private. Renaming a state directory
would also transfer consent and is not a supported migration procedure.

## Names in historical records

The repository was called `middle-finger` (sometimes written `middle-figner`)
and then `craig`. Git history, benchmark evidence, and existing synthetic corpus
provenance retain the names recorded at their creation. They do not identify
current commands or installation paths. `npm run branding:verify` checks active
source and filenames while allowing these documented historical references.
