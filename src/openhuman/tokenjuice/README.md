# tokenjuice

Terminal-output compaction engine. A Rust port of [vincentkoc/tokenjuice](https://github.com/vincentkoc/tokenjuice) that shrinks verbose tool/command output (git, npm, cargo, docker, kubectl, lint, test runners, …) **before** it enters an LLM context window. Given a tool invocation (tool name, argv/command, stdout/stderr, exit code), it classifies the output against a JSON-configured rule set, applies filtering / summarisation / counting transforms, and returns a compacted inline string plus reduction stats. It is a **pure library**: no JSON-RPC surface, no CLI, no persistence, no event bus. The only outside consumer is the agent tool loop, which calls `compact_tool_output` after each successful tool call.

## Responsibilities

- Normalise a `ToolExecutionInput` (derive `argv` from `command` via a small shell tokenizer when absent).
- Classify the input against a rule set: filter rules by `match` criteria, score by specificity, pick the best (or honour a forced rule id), else `generic` / `generic/fallback`.
- Apply the matched rule's pipeline: pretty-print JSON, strip ANSI, skip/keep line filters, trim empty edges, dedupe adjacent lines, head/tail summarisation, pattern counters, `onEmpty` and output-match canned messages.
- Run special-case post-processors for `git/status` (porcelain rewrite to `M:`/`A:`/`D:`/`R:`/`??`) and `cloud/gh` (JSON-record / table row formatting).
- Failure-aware summarisation: when `exit_code != 0` and a rule has `failure.preserveOnFailure`, use the wider `failure.head`/`failure.tail` window.
- Decide between compacted vs passthrough output (tiny outputs ≤240 chars and file-inspection commands like `cat`/`sed`/`jq` are returned verbatim) and clamp to `max_inline_chars` (default 1200) with end- or middle-truncation.
- Load/compile rules from a three-layer overlay (builtin → user → project), precompiling all regex at load time.
- Provide a pass-through-safe agent glue (`compact_tool_output`) that only substitutes the compacted text when it is meaningfully smaller (ratio ≤ 0.95 and below 512-byte input is skipped entirely).

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/tokenjuice/mod.rs` | Module docstring + public re-exports. Declares submodules. No logic. |
| `src/openhuman/tokenjuice/types.rs` | All serde types mirroring upstream shapes: `JsonRule` + sub-types (`RuleMatch`, `RuleFilters`, `RuleTransforms`, `RuleSummarize`, `RuleCounter`, `RuleOutputMatch`, `RuleFailure`, `CounterSource`), compiled forms (`CompiledRule`, `CompiledParts`, `CompiledCounter`, `CompiledOutputMatch`, `RuleOrigin`), I/O types (`ToolExecutionInput`, `ReduceOptions`, `CompactResult`, `ReductionStats`, `ClassificationResult`, `RuleFixture`). |
| `src/openhuman/tokenjuice/reduce.rs` | The main pipeline: `reduce_execution_with_rules`, command tokenization/normalisation, git-status and gh post-processors, JSON pretty-print, `apply_rule`, passthrough/inline selection, char clamping. Thread-local regex cache for hot per-line patterns. |
| `src/openhuman/tokenjuice/classify.rs` | `matches_rule`, `score_rule`, `classify_execution` — rule matching + specificity scoring. |
| `src/openhuman/tokenjuice/tool_integration.rs` | Agent glue: `compact_tool_output` + `CompactionStats`, lazily-cached builtin rule set, `extract_command_argv` for shell-shaped tool arguments. |
| `src/openhuman/tokenjuice/rules/mod.rs` | Re-exports `compile_rule`, `load_builtin_rules`, `load_rules`, `LoadRuleOptions`. |
| `src/openhuman/tokenjuice/rules/loader.rs` | Three-layer overlay loader (builtin/user/project), recursive `.json` discovery, id-keyed overlay merge, fallback-last sort. |
| `src/openhuman/tokenjuice/rules/compiler.rs` | `compile_rule`: builds `regex::Regex` (translating JS `i`/`m` flags to inline flags); drops invalid regex non-fatally. |
| `src/openhuman/tokenjuice/rules/builtin.rs` | `BUILTIN_RULE_JSONS`: `(id, include_str!)` table of all vendored rules embedded at compile time. |
| `src/openhuman/tokenjuice/text/mod.rs` | Re-exports text helpers. |
| `src/openhuman/tokenjuice/text/process.rs` | Line ops: `normalize_lines`, `trim_empty_edges`, `dedupe_adjacent`, `head_tail`, `clamp_text`, `clamp_text_middle`, `pluralize`. |
| `src/openhuman/tokenjuice/text/ansi.rs` | `strip_ansi`. |
| `src/openhuman/tokenjuice/text/width.rs` | `count_text_chars`, `count_terminal_cells`, `graphemes` (Unicode-aware). |
| `src/openhuman/tokenjuice/vendor/rules/*.json` | 96 vendored upstream rule JSON files (`family__name.json` naming), embedded via `builtin.rs`. |
| `src/openhuman/tokenjuice/vendor/README.md` | Provenance + MIT licence of vendored upstream rules; exclusions and how to add rules. |
| `src/openhuman/tokenjuice/tests/fixtures/*.fixture.json` | `RuleFixture` integration-test fixtures (cargo test failure, fallback long output, git status). |
| `*_tests.rs` (`reduce_tests.rs`, `text_tests.rs`, `rules/builtin_tests.rs`, `rules/loader_tests.rs`) | Sibling `#[path]` test suites; plus inline `#[cfg(test)]` tests in `classify.rs` / `tool_integration.rs`. |

## Public surface

Re-exported from `mod.rs`:

- `reduce_execution_with_rules(input, rules, opts) -> CompactResult` — synchronous core pipeline against a pre-loaded rule set.
- `load_builtin_rules() -> Vec<CompiledRule>` — embedded rules only (no disk I/O); `load_rules(&LoadRuleOptions)` for the full builtin/user/project overlay.
- `compact_tool_output(tool_name, arguments, output, exit_code) -> (String, CompactionStats)` — the agent-facing, pass-through-safe entry point.
- Types: `CompactResult`, `ReduceOptions`, `ToolExecutionInput`, `CompactionStats`, `LoadRuleOptions`.

## Persistence

None at runtime. Builtin rules are embedded at compile time. Optionally reads rule JSON from disk at load time (not written): user layer `~/.config/tokenjuice/rules/` and project layer `<cwd>/.tokenjuice/rules/` (both overridable / skippable via `LoadRuleOptions`). `compact_tool_output` uses only the builtin set.

## Dependencies

This module is **fully self-contained within `openhuman`** — it has no `use crate::openhuman::` or `use crate::core::` imports on any other domain or transport module. External crate dependencies only:

- `serde` / `serde_json` — rule and I/O (de)serialisation; gh-output JSON parsing.
- `regex` — rule pattern matching (precompiled at load; thread-local cache for ad-hoc per-line patterns in `reduce.rs`).
- `once_cell::sync::Lazy` — lazy builtin rule cache and the compiled gh table-split regex.
- `dirs` — resolves the user home for the user-layer rules directory.
- `unicode-segmentation` — grapheme-aware width/char counting in `text/width.rs`.
- `log` — verbose `[tokenjuice]`-prefixed diagnostics throughout.

## Used by

- `src/openhuman/agent/harness/tool_loop.rs` — the only consumer. Calls `crate::openhuman::tokenjuice::compact_tool_output(&call.name, Some(&call.arguments), &scrubbed, Some(0))` after a successful tool call (post credential-scrub) to compact tool output before it is fed back to the model.

## Notes / gotchas

- **Library-only by design** (v1). No RPC/CLI/store/bus surfaces; the module docstring states they can be layered on later.
- `generic/fallback` rule **must** be present in any rule set — `reduce_execution_with_rules` `expect`s it, and the loader always sorts it last so it never shadows a more specific rule.
- Pass-through safety: `compact_tool_output` returns the untouched original (and `stats.applied == false`) for inputs < 512 bytes or when compaction ratio > 0.95 — callers never need to guard the call site, and data is never silently lost.
- `git/status` and `cloud/gh` carry **hard-coded** post-processors in `reduce.rs` keyed off `rule.id`, beyond what the JSON rules express.
- JS→Rust regex flag translation: only `i` and `m` are honoured (as inline `(?i)`/`(?m)`); Unicode is always on in Rust's `regex` (no separate `u` flag). Invalid regex in a rule is logged and dropped, not fatal.
- Disk-loaded rule files ending in `.schema.json` or `.fixture.json` are excluded from discovery; symlinks are skipped.
- Vendored rules exclude upstream's `openclaw/` subdirectory (proprietary, non-generic) — see `vendor/README.md`. To add a rule, drop the JSON in `vendor/rules/` (`family__name.json`) and add the `(id, include_str!)` entry to `builtin.rs`.
