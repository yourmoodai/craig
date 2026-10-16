# Commit message checks

Adapted from [awill1988/okerrr](https://github.com/awill1988/okerrr/tree/main/tools/commit_check)
under the retained MIT license. This copy enforces 50-character subjects and
72-character body lines, rejects attribution without exceptions, and skips
merge wrapper messages while checking their introduced non-merge commits.

```sh
cargo run -p commit_check -- --edit /path/to/commit-message
cargo run -p commit_check -- --range origin/main HEAD
cargo run --release -p commit_check -- --edit /path/to/commit-message
cargo test -p commit_check
```

Input without arguments is read from stdin. Git hooks and CI invoke the same
validator. Existing published bootstrap history predates this policy; CI starts
new-commit checks after that checkpoint. The checker is a metadata linter.
