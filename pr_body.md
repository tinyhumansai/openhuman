## Summary

- Added `tokenjuice` reduction rules for common Rust toolchain commands to improve AI agent parsing efficiency.
- Supported commands: `cargo clippy`, `cargo build` (and `cargo check`), `cargo fmt --check`, and `cargo doc`.
- Ensures warnings, errors, and diff hunks are preserved while stripping redundant compilation noise (e.g. `Compiling`/`Checking`/`Documenting` progress lines).

## Problem

- Rust projects generate highly verbose terminal output during compilation and linting.
- The `tokenjuice` engine was missing specific rules for the Rust toolchain (except `cargo-test`), leading to large token counts being fed into LLM prompts when agents ran `cargo build` or `cargo clippy`.
- Raw output from these commands often exceeded token limits and cluttered context with irrelevant dependency compilation logs.

## Solution

- Added four new JSON rules (`lint__cargo-clippy.json`, `build__cargo-build.json`, `lint__cargo-fmt.json`, `build__cargo-doc.json`) using the vendor reduction pattern.
- Registered the new rules in `src/openhuman/tokenjuice/rules/builtin.rs` and updated expected rule counts.
- Added comprehensive classification and reduction integration tests in `classify.rs` and `reduce_tests.rs` verifying that `rustc` diagnostic errors/warnings and `cargo fmt` diff hunks are preserved while noisy `Compiling ...` lines are filtered out.

## Submission Checklist

> If a section does not apply to this change, mark the item as `N/A` with a one-line reason. Do not delete items.

- [x] Tests added or updated (happy path + at least one failure / edge case) per [Testing Strategy](../gitbooks/developing/testing-strategy.md#failure-path-requirement)
- [x] **Diff coverage ≥ 80%** — changed lines (Vitest + cargo-llvm-cov merged via `diff-cover`) meet the gate enforced by [`.github/workflows/coverage.yml`](../.github/workflows/coverage.yml). Run `pnpm test:coverage` and `pnpm test:rust` locally; PRs below 80% on changed lines will not merge.
- [ ] Coverage matrix updated — N/A: pure reduction rule addition, doesn't modify overarching feature flags.
- [ ] All affected feature IDs from the matrix are listed in the PR description under `## Related`
- [x] No new external network dependencies introduced (mock backend used per [Testing Strategy](../gitbooks/developing/testing-strategy.md#mock-policy))
- [ ] Manual smoke checklist updated if this touches release-cut surfaces — N/A: no UI/release surfaces modified.
- [ ] Linked issue closed via `Closes #NNN` in the `## Related` section — N/A

## Impact

- Improves agent reliability and efficiency when interacting with Rust repositories.
- Minor token processing improvement for `tokenjuice` by pre-compiling 4 new Regex rules on startup.

## Related

- Closes: N/A
- Follow-up PR(s)/TODOs: N/A

---

## AI Authored PR Metadata (required for Codex/Linear PRs)

> Keep this section for AI-authored PRs. For human-only PRs, mark each field `N/A`.

### Linear Issue
- Key: N/A
- URL: N/A

### Commit & Branch
- Branch: feat/rust-tokenjuice-rules
- Commit SHA: N/A

### Validation Run
- [x] `pnpm --filter openhuman-app format:check`
- [x] `pnpm typecheck`
- [x] Focused tests: `cargo test -p openhuman --lib -- tokenjuice`
- [x] Rust fmt/check (if changed): N/A
- [x] Tauri fmt/check (if changed): N/A

### Validation Blocked
- `command:` N/A
- `error:` N/A
- `impact:` N/A

### Behavior Changes
- Intended behavior change: AI agents will now receive compacted, noise-free output for `cargo build/check/clippy/fmt/doc` commands.
- User-visible effect: Faster agent responses and lower token usage on Rust projects.

### Parity Contract
- Legacy behavior preserved: N/A
- Guard/fallback/dispatch parity checks: N/A

### Duplicate / Superseded PR Handling
- Duplicate PR(s): N/A
- Canonical PR: N/A
- Resolution (closed/superseded/updated): N/A
