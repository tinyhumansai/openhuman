# Agent runtime helper migration packages

> **Status, verified 2026-10-07:** landed. See [`README.md`](README.md) for
> the whole set and how to re-check it.

These tasks extend Task 2 in
[`migrate-agent-runtime-to-tinyagents.md`](migrate-agent-runtime-to-tinyagents.md).
Every GREEN step includes direct consumer imports and deletion of the old path;
facades and re-exports in OpenHuman or TinyAgents are forbidden.

## Task 2a: Move context statistics and prompt mechanics

**Files:** TinyAgents harness `src/context/` and `src/prompt/`; OpenHuman
`agent/context/`, especially `agent/context/stats.rs`, and direct consumers.

**RED:** Port fixtures that calculate message/tool/image/token statistics,
preserve tool-call/result pairing, assemble ordered prompt sections, enforce
section budgets, and report truncation provenance. Confirm the absent APIs fail:

```bash
cargo test --manifest-path vendor/tinyagents/Cargo.toml -p tinyagents-harness context_stats
cargo test --manifest-path vendor/tinyagents/Cargo.toml -p tinyagents-harness prompt_mechanics
```

**GREEN:** Move only host-free statistics and prompt assembly mechanics. Accept
message, section, tokenizer, and budget inputs through crate-native types or
callbacks; do not name OpenHuman context, memory, profile, or prompt assets.
Direct-import the owner everywhere, delete the OpenHuman helpers and
`context/prompt.rs`, then re-run both focused tests, `cargo check -p openhuman`,
and the boundary checker.

## Task 2b: Move behavior-free prompt render helpers

**Files:** TinyAgents harness `src/prompt/`; OpenHuman `agent/prompts/`,
`agent/harness/instructions.rs`, tests, and callers.

**RED:** Port exact-output fixtures for headings, optional/empty sections,
escaping, tool catalog blocks, stable ordering, and byte/token caps. Use only
caller-supplied content, with no OpenHuman product copy.

```bash
cargo test --manifest-path vendor/tinyagents/Cargo.toml -p tinyagents-harness prompt_render
```

**GREEN:** Move reusable render blocks to the harness. Keep product prompts and
product section selection in OpenHuman. Direct-import every helper, delete the
forwarding functions/modules, and re-run the focused test, root cargo check,
and boundary checker. No TinyAgents crate may re-export the helpers.

## Task 2c: Direct-import multimodal marker, MIME, and data-URI helpers

**Files:** TinyAgents harness multimodal module; OpenHuman
`agent/multimodal.rs`, attachment resolution, model adapters, and tests.

**RED:** Port table-driven tests for marker recognition, case-insensitive MIME
parsing, supported/unsupported media, base64 and percent-encoded data URIs,
malformed payload rejection, size limits, and text/image ordering. Add a host
test proving authorization happens before file bytes reach the generic helper.

```bash
cargo test --manifest-path vendor/tinyagents/Cargo.toml -p tinyagents-harness multimodal
pnpm debug rust multimodal
```

**GREEN:** Move pure marker/MIME/data-URI parsing and rendering upstream. Keep
file access, attachment lookup, authorization, and size policy in OpenHuman.
Direct-import the owner, delete old exports, and re-run both focused commands,
root cargo check, and the boundary checker.

## Task 2d: Move embedding and retriever interfaces

**Files:** TinyInference embedding traits/tests; TinyAgents harness retriever
traits/tests; OpenHuman `agent/tinyagents/{embeddings,retriever}.rs`,
provider/config, memory adapters, and tests.

**RED:** Add provider-neutral tests for batched embeddings, stable input/result
order, dimension/usage metadata, empty/error responses, cancellation,
retrieval limits/scores/metadata, and a fake retriever in context composition.
Add OpenHuman tests proving provider configuration and memory scope stay host
owned.

```bash
cargo test --manifest-path vendor/tinyagents/vendor/tinyinference/Cargo.toml --workspace embedding
cargo test --manifest-path vendor/tinyagents/Cargo.toml -p tinyagents-harness retriever
pnpm debug rust retriever
```

**GREEN:** Put model-boundary embedding types/traits in TinyInference and
generic retrieval contracts in the harness. Keep provider construction,
credentials, memory queries, scope, and policy in OpenHuman adapters.
Direct-import owners, delete both OpenHuman facades, and re-run all three
commands, root cargo check, and the boundary checker.
