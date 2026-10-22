# Local replacement inspection

A replacement trace records actual detector spans, applied edits, configuration
identity, processing time, and the adapter's outcome. Human judgments are
separate annotations, not detector scores or provider approval.

## Commands

From a source build:

```sh
npm run build
node bridge/cli.mjs inspect
node bridge/cli.mjs traces status
node bridge/cli.mjs traces enable
node bridge/cli.mjs traces disable
node bridge/cli.mjs traces export /absolute/path/replacements.parquet
node bridge/cli.mjs traces delete
```

Open the printed loopback URL in a browser. Its fragment contains a per-launch
access token; do not share the URL. The app removes the token from the address
bar, so refresh requires reopening the original URL. `Ctrl+C` stops the server.
No background service is installed. An installed plugin can invoke its
`bridge/cli.mjs` using the bundled `runtime/<target>/node` executable instead of
PATH Node. The bridge prepares verified runtime artifacts when necessary.

Full-text capture defaults off. Enable it through the command or the web app's
confirmation before submitting prompts. Disabling stops new text capture;
deletion separately disables capture and removes retained records. Exports may
contain secrets and remain after deletion. Files have private permissions but
are not encrypted. Operating-system backups are outside Craig's control.

## Storage and interpretation

On Unix, downloads use `$XDG_CACHE_HOME/craig` (default `~/.cache/craig`), and
traces use `$XDG_STATE_HOME/craig/profiles/<opaque-id>` (default
`~/.local/state/craig/profiles/<opaque-id>`). Windows uses private directories
under `%LOCALAPPDATA%/craig`. `CLAUDE_CONFIG_DIR` selects the profile; otherwise
the default `.claude` location identifies it. Absolute paths are required;
symlinked storage paths are rejected.

Retention is seven days with a 256 MiB storage ceiling. Conservative reservations
for database maintenance and snapshots can evict older records sooner. Expiry
runs when capturing or inspecting records; stopped software cannot enforce a
wall-clock deletion deadline. Existing text-free personal-review metrics remain
in their original store and keep their separate consent and retention policy.

`traces.sqlite3` is the canonical journal. `traces.parquet` is a revision-bound
projection rebuilt only after data changes, with the same event IDs and joined
annotations. Export columns are `schema`, `id`, `timestamp_ms`, `outcome`, `event`,
and `annotation`. The latter two contain versioned JSON; trace spans use UTF-8
byte offsets. Opening the inspector does not rerun detection or recover old
originals from Claude history.

`processed` means local rewriting completed. `forwarded` means the adapter's
downstream call returned; it does not prove provider receipt. Failed or cancelled
attempts have no successful output. Adapter-level failures without a worker
result retain unavailable model identity rather than an invented one. A recording
gap warns without changing the successful replacement. Capture is currently
wired to the Claude adapter's version-3 bridge requests and the explicit native
`rewriteTraced` method; existing version-1/2 callers do not gain persistence.

## Verification

```sh
npm run inspect:build:debug
npm run inspect:build:release
npm run inspect:test
npm run test:bootstrap
npx playwright install chromium
npm run inspect:e2e
npm run inspect:profile
npm run verify
```

Both `inspect:e2e` and `inspect:profile` accept a new JSON output path after `--`
to retain source-bound evidence. They refuse to overwrite an existing report.
The native release binary also exposes `craig inspect` and `craig traces`; it
delegates to the adjacent, verified `craig-review` executable.

The browser check uses synthetic prompts, isolated state, and the current local
model. It checks consent, exact Unicode edits, annotation, literal hostile markup,
authentication, Host/Origin restrictions, export, deletion, and local-only browser
requests. Profiling reports five short-input samples for disabled, metadata, and
full-text capture, RSS, and cold/warm export times. It is not detector latency
qualification. Editor tasks expose debug/release inspection, tests, browser E2E,
and memory profiling.

Signed-release installation is a separate gate: candidate cache tests do not
prove a public GitHub download works. Production signing-key provisioning and
the macOS/Linux/Windows published-download matrix are required before marketplace
promotion. The bootstrap recovers a lock whose recorded PID is no longer alive.
An incomplete lock or interrupted recovery can require manual repair: confirm
no setup process is running before removing only the lock in the identified
release cache. Verified partial downloads remain reusable.
