# util/

Shared utility helpers used across the memory-tree subsystem. Kept pure-function and dependency-light so any module in `tree/` can pull them in without cycle risk.

## Files

- [`mod.rs`](mod.rs) — module banner; re-exports `redact`.
- [`redact.rs`](redact.rs) — log-time PII redaction. `redact(s)` hashes a string to 8 stable hex chars (safe to grep when the raw value is available externally). `redact_endpoint(url)` strips scheme, path, query, fragment, and credentials, keeping only `host[:port]`.

## When to use

Per CLAUDE.md: never log secrets or full PII. After the participant-bucketing change, source_ids and content_paths can embed full email addresses, so any log line that prints them must redact first.
