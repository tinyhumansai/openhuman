---
name: cargo-check-vs-test-verification
description: cargo check passing does NOT mean tests compile; always cargo test --no-run to verify test modules
metadata:
  type: feedback
---

`cargo check --manifest-path Cargo.toml` compiles the **lib only** — it does NOT compile `#[cfg(test)]` modules or sibling `*_tests.rs` files. A domain can pass `cargo check` while its own test modules have compile errors (missing imports in `use super::*` test files, private-fn re-export E0364, missing struct fields in test-only `PromptContext` construction sites, etc.).

**Why:** In this repo, `cargo check` greenlit a new domain whose `select_tests.rs` had unresolved `WorkflowPhase`/`PHASE_*` imports and whose `ops.rs` had a `pub(crate) use slugify` (E0364) — all invisible until `cargo test` compiled the test cfg. Also: adding a field to a widely-constructed struct (e.g. `PromptContext`) requires updating **every** construction site including those in `tests/*.rs` integration tests (e.g. `tests/personality_e2e.rs`), which `cargo check` won't surface.

**How to apply:** To verify Rust work actually compiles AND tests are valid, run `cargo test --manifest-path Cargo.toml --no-run` (compiles all test targets) and then run the actual tests. Never report "N tests pass" based on a `cargo check` exit code or a `cargo test <filter>` run that shows "0 passed; N filtered out" — that means the filter matched nothing, not success. Confirm the result line shows a non-zero passed count. See [[subagent-output-can-be-lost]].
