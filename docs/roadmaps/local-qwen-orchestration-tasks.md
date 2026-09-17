# Phase 7 worker execution registry

This is the scoped execution registry for Phase 7 of
`local-qwen-orchestration.md`. The roadmap is authoritative. This registry
does not alter requirements, gate order, or acceptance. SOL owns discovery,
architecture, task extraction, worktrees, review, integration, gate checks,
live execution, packaging, commits, and push. Implementation patches are
delegated to isolated Antigravity workers using model
`gemini-3.8-flash-high`.

## Worker isolation contract

Each worker receives a generated `.agent/TASK.md` no larger than 4 KiB. It may
change only the one or two paths named by its task, read only the listed context
paths, and run only the listed checks. It must not read `AGENTS.md`, either
roadmap, other task records, or unspecified repository files; investigate or
redesign; spawn another worker; run broad suites; commit to the integration
branch; or fix incidental findings. A broader need is a blocker returned to
SOL. Automated tests use mocks and make no real backend or third-party calls.

Workers preserve the existing model binding `lmstudio:qwen38-openhuman`, LM
Studio endpoint `http://127.0.0.1:1234/v1`, prompt-guided tool mode, chat
template, and `131072` context. They do not weaken OpenHuman security,
approvals, action/workspace boundaries, persistence, progress events, or UI.
Goose remains the only agent loop; Qwen code is a provider dialect adapter.

## Settled architecture

- Typed primary capability policy lives under
  `agent/primary_orchestration/`; it is a sidecar over OpenHuman tools and does
  not change the shared `tinytools::Tool` trait.
- `ToolCapability` contains name, ordered operations and modalities, backend,
  monetary boundary, side effect, availability, permission, priority, and
  registration index. `CapabilityPlan.routes` is authoritative; a derived
  name set may only enforce calls.
- Exact-name tables classify known tools. No substring or schema-description
  routing is permitted. An unclassified tool is diagnostic-only and disabled,
  never model-visible.
- Route order is `(backend rank, priority, registration index)`: local,
  local-browser, free direct-network, configured BYOK, then explicitly opted-in
  OpenHuman-managed. Fallback may not cross operation, modality, permission,
  side-effect, or monetary boundaries.
- Persisted `agent.allow_metered_agent_tools` defaults false. A per-turn RPC
  value may only reduce that permission. Authentication never enables it.
- Tool registration establishes readiness; explicit health inputs may narrow
  it. Missing/unhealthy memory is absent from unrelated turns and produces one
  deterministic unavailable result only for explicit memory requests.
- Goose receives the ordered selected registry, while its provider reuses the
  existing approval/security/execution path and durable effect claims.
- The Qwen adapter runs only for the exact local binding. It normalizes the
  complete response after stream assembly, prefers native structured calls,
  accepts one action, strips raw protocol text, persists correction count, and
  never chooses tools or executes effects.
- Completion and loop guards are typed persisted state evaluated by Goose
  operations. Informational observations normally require a final response;
  only an explicitly complete typed result may use deterministic rendering.

## Gate 4 — typed capabilities and monetary policy

Tasks in the same wave are disjoint and may run concurrently. Tasks touching a
shared module file run only after the preceding integration commit.

### G4-A — capability vocabulary (wave 4.0, lane capability)

- Depends: Gate 3.
- Change: `crates/openhuman-core/src/agent/primary_orchestration/capability.rs`;
  `crates/openhuman-core/src/agent/primary_orchestration/mod.rs`.
- Context: `crates/openhuman-core/src/tools/traits.rs`;
  `crates/openhuman-core/src/agent/primary_orchestration/mode.rs`.
- Implement the settled enums and `ToolCapability`, `ToolRoute`,
  `CapabilityPlan`, `CapabilityPolicy`, validation errors, backend rank, stable
  route ordering, and derived enforcement-name set. Declare an external test
  module path in `capability.rs` without inline tests.
- Verify: `cargo check --manifest-path Cargo.toml -p openhuman --lib`;
  `git diff --check`.

### G4-B — capability vocabulary tests (wave 4.1, lane capability)

- Depends: G4-A.
- Change: `crates/openhuman-core/src/agent/primary_orchestration/capability_tests.rs`.
- Context: `crates/openhuman-core/src/agent/primary_orchestration/capability.rs`.
- Cover enum serialization, all required fields, duplicate rejection, stable
  registration order, backend ordering, priority ties, and derived-set parity.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib capability_tests`;
  `git diff --check`.

### G4-AX — capability Clippy canonicalization (wave 4.1, lane capability)

- Depends: G4-A.
- Change: `crates/openhuman-core/src/agent/primary_orchestration/capability.rs`.
- Context: None.
- Express backend/monetary compatibility with `matches!` so the repository's
  `-D warnings` Clippy gate passes without changing the accepted pairs.
- Verify: `cargo clippy --manifest-path Cargo.toml -p openhuman --lib -- -D warnings`;
  `git diff --check`.

### G4-C — persisted metered opt-in (wave 4.0, lane configuration)

- Depends: Gate 3.
- Change: `crates/openhuman-core/src/config/schema/agent.rs`;
  `crates/openhuman-core/src/config/schema/agent_memory_window_tests_tests.rs`.
- Context: `crates/openhuman-core/src/config/schema/types/config.rs`.
- Add `allow_metered_agent_tools: bool` with serde default false and schema
  docs stating sign-in is not authorization. Prove missing/empty/round-trip
  config stays false and explicit true persists.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib agent_memory_window_tests`;
  `git diff --check`.

### G4-D — metered setting operations (wave 4.1, lane configuration)

- Depends: G4-C.
- Change: `crates/openhuman-core/src/config/ops/agent.rs`;
  `crates/openhuman-core/src/config/ops_voice_and_autonomy_tests.rs`.
- Context: `crates/openhuman-core/src/config/schema/agent.rs`.
- Extend `AgentSettingsPatch`, apply/get payloads, persistence, and sanitized
  logs with the boolean. Omission leaves the saved value unchanged.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib apply_agent_settings`;
  `git diff --check`.

### G4-DX — setting-patch literal compatibility (wave 4.2, lane tests)

- Depends: G4-D.
- Change: `crates/openhuman-core/src/config/ops_agent_paths_tests.rs`.
- Context: `crates/openhuman-core/src/config/ops/agent.rs`.
- Update the two timeout-only `AgentSettingsPatch` literals to use the default
  for newly added optional settings without changing their assertions.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib ops_agent_paths_tests`;
  `git diff --check`.

### G4-E1 — metered setting RPC wire (wave 4.2, lane configuration)

- Depends: G4-D, G4-DX.
- Change: `crates/openhuman-core/src/config/schemas/controllers/agent.rs`;
  `crates/openhuman-core/src/config/schemas/helpers.rs`.
- Context: `crates/openhuman-core/src/config/ops/agent.rs`;
  `crates/openhuman-core/src/config/schemas/controllers_tests.rs`.
- Deserialize optional `allow_metered_agent_tools` and forward it mechanically
  to the settings patch. Non-booleans remain on the existing structured
  invalid-params path.
- Verify: `cargo check --manifest-path Cargo.toml -p openhuman --lib`;
  `git diff --check`.

### G4-E2 — metered setting RPC schema (wave 4.3, lane configuration)

- Depends: G4-E1.
- Change: `crates/openhuman-core/src/config/schemas/schema_defs/agent.rs`;
  `crates/openhuman-core/src/config/schemas_tests.rs`.
- Context: `crates/openhuman-core/src/config/schemas/helpers.rs`;
  `crates/openhuman-core/src/config/ops/agent.rs`.
- Document optional `allow_metered_agent_tools`, including the persisted
  opt-in and sign-in boundary, and assert its optional boolean schema.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib config::schemas`;
  `git diff --check`.

### G4-F — downward per-turn monetary override wire (wave 4.0, lane RPC)

- Depends: Gate 3.
- Change: `crates/openhuman-core/src/web_chat/types.rs`;
  `crates/openhuman-core/src/web_chat/schemas.rs`.
- Context: `crates/openhuman-core/src/web_chat/run_task.rs`.
- Add optional `allow_metered_tools` to RPC input and turn metadata. It means
  downward override only; false forbids managed routes and true never broadens
  persisted false. Preserve all existing request defaults.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib web_chat::schemas`;
  `git diff --check`.

### G4-G — typed request intent (wave 4.2, lane capability)

- Depends: G4-A, G4-B.
- Change: `crates/openhuman-core/src/agent/primary_orchestration/intent.rs`;
  `crates/openhuman-core/src/agent/primary_orchestration/mod.rs`.
- Context: `crates/openhuman-core/src/agent/primary_orchestration/mode.rs`;
  `crates/openhuman-core/src/agent/harness/primary_tool_exposure.rs`.
- Implement ordered data rules yielding requested operations, modalities,
  completion kind, known URL, and explicit memory/generation/retrieval flags.
  Prompt matching may classify user intent but must never match tool names or
  descriptions. Ambiguity narrows. Declare an external test module.
- Verify: `cargo check --manifest-path Cargo.toml -p openhuman --lib`;
  `git diff --check`.

### G4-H — request-intent tests (wave 4.3, lane capability)

- Depends: G4-G.
- Change: `crates/openhuman-core/src/agent/primary_orchestration/intent_tests.rs`.
- Context: `crates/openhuman-core/src/agent/primary_orchestration/intent.rs`;
  `crates/openhuman-core/src/agent/primary_orchestration/mode.rs`.
- Cover greeting, current news, known URL, image retrieval with a negative
  generation instruction, image generation, explicit recall/store, repository
  mutation, scheduling, and ambiguous downward behavior.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib intent_tests`;
  `git diff --check`.

### G4-GX — external intent-test module seam (wave 4.3, lane capability)

- Depends: G4-H.
- Change: `crates/openhuman-core/src/agent/primary_orchestration/intent.rs`.
- Context: None.
- Point the existing test module declaration at the registered sibling
  `intent_tests.rs`; do not change intent behavior or tests.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib intent_tests`;
  `git diff --check`.

### G4-GY — intent URL trim clippy seam (wave 4.7, lane capability)

- Depends: G4-GX.
- Change: `crates/openhuman-core/src/agent/primary_orchestration/intent.rs`.
- Context: None.
- Replace the manual start/end punctuation character comparisons in URL token
  trimming with equivalent character-array patterns. Preserve the exact
  punctuation set and resolver behavior.
- Verify: `cargo clippy --manifest-path Cargo.toml -p openhuman --lib -- -D warnings`;
  `git diff --check`.

### G4-I — exact capability catalog (wave 4.3, lane catalog)

- Depends: G4-A, G4-G.
- Change: `crates/openhuman-core/src/agent/primary_orchestration/catalog.rs`;
  `crates/openhuman-core/src/agent/primary_orchestration/mod.rs`.
- Context: `crates/openhuman-core/src/tools/ops.rs`;
  `crates/openhuman-core/src/tools/user_filter.rs`;
  `crates/openhuman-core/src/search/registry.rs`;
  `crates/openhuman-core/src/tools/traits.rs`.
- Build descriptors in registration order using exact-name tables. Cover local
  workspace/system, browser, free direct network, each BYOK search family,
  managed search/media/integrations, memory, schedule, and delegation. Derive
  permission and local/external effect conservatively from the tool. Unknown
  names become disabled with a diagnostic. Declare an external test module.
- Verify: `cargo check --manifest-path Cargo.toml -p openhuman --lib`;
  `git diff --check`.

### G4-J — catalog and health tests (wave 4.4, lane catalog)

- Depends: G4-I.
- Change: `crates/openhuman-core/src/agent/primary_orchestration/catalog_tests.rs`.
- Context: `crates/openhuman-core/src/agent/primary_orchestration/catalog.rs`;
  `crates/openhuman-core/src/tools/ops_tests.rs`;
  `crates/openhuman-core/src/search/registry_tests.rs`.
- Assert representative registered families receive exact typed metadata,
  registration order is preserved, duplicates fail, unclassified tools are
  disabled, absent memory is not advertised, and managed entries are correctly
  marked regardless of authentication.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib catalog_tests`;
  `git diff --check`.

### G4-K — ordered capability planner (wave 4.4, lane planner)

- Depends: G4-H, G4-I.
- Change: `crates/openhuman-core/src/agent/primary_orchestration/planner.rs`;
  `crates/openhuman-core/src/agent/primary_orchestration/mod.rs`.
- Context: `crates/openhuman-core/src/agent/primary_orchestration/capability.rs`;
  `crates/openhuman-core/src/agent/primary_orchestration/intent.rs`;
  `crates/openhuman-core/src/agent/primary_orchestration/catalog.rs`.
- Intersect readiness, session ceiling, mode, operations/modalities,
  permission, side-effect, and effective monetary policy. Preserve ordered
  routes; never invent fallback or cross a boundary. Return a deterministic
  unavailable observation when an explicit request has no route. Declare an
  external test module.
- Verify: `cargo check --manifest-path Cargo.toml -p openhuman --lib`;
  `git diff --check`.

### G4-L — planner policy matrix (wave 4.5, lane planner)

- Depends: G4-K.
- Change: `crates/openhuman-core/src/agent/primary_orchestration/planner_tests.rs`.
- Context: `crates/openhuman-core/src/agent/primary_orchestration/planner.rs`;
  `crates/openhuman-core/src/agent/primary_orchestration/capability.rs`.
- Cover chat zero tools; local/browser/free/BYOK/managed ordering; persisted
  opt-in and downward override truth table; disabled paid tools; security and
  health filtering; retrieval/generation separation; memory readiness;
  duplicate rejection; and forbidden cross-boundary fallback.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib planner_tests`;
  `git diff --check`.

### G4-M — ordered Goose tool registry (wave 4.5, lane Goose)

- Depends: G4-K.
- Change: `crates/openhuman-core/src/agent/goose/tools.rs`;
  `crates/openhuman-core/src/agent/goose/runner.rs`.
- Context: `crates/openhuman-core/src/agent/primary_orchestration/capability.rs`;
  `crates/openhuman-core/src/agent/harness/session/types.rs`.
- Replace the adapter's owned `Vec<Arc<dyn Tool>>` with a registry that holds
  the agent's existing durable/synthesized `Arc<Vec<Box<dyn Tool>>>` snapshots
  plus ordered routes. Advertise in route order and enforce the derived name
  set before existing approval, durable claim, and execution. Do not duplicate
  policy or clone tool implementations.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib agent::goose::tests`;
  `git diff --check`.

### G4-MX — Goose registry compile seam (wave 4.5, lane integration)

- Depends: G4-M.
- Change: `crates/openhuman-core/src/agent/harness/session/runtime/primary_turn.rs`.
- Context: `crates/openhuman-core/src/agent/goose/runner.rs`.
- Update the dormant fail-closed Goose adapter construction for the ordered
  registry fields introduced by G4-M. Keep both tool snapshots and routes empty;
  G4-O remains responsible for building the real immutable capability plan.
- Verify: `cargo check --manifest-path Cargo.toml -p openhuman --lib`;
  `git diff --check`.

### G4-N — Goose registry enforcement tests (wave 4.6, lane Goose)

- Depends: G4-MX.
- Change: `crates/openhuman-core/src/agent/goose/tests.rs`.
- Context: `crates/openhuman-core/src/agent/goose/tools.rs`;
  `crates/openhuman-core/src/agent/goose/runner.rs`;
  `crates/openhuman-core/src/agent/primary_orchestration/capability.rs`.
- Prove prompt order equals route order, omitted tools cannot execute, approval
  and durable effect claims remain mandatory, and route metadata cannot widen
  the underlying security ceiling.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib agent::goose::tests`;
  `git diff --check`.

### G4-NX — Goose rejection outcome assertions (wave 4.6, lane Goose)

- Depends: G4-N.
- Change: `crates/openhuman-core/src/agent/goose/tests.rs`.
- Context: `crates/openhuman-core/src/agent/goose/tools.rs`.
- Assert omitted and unplanned calls through Goose's fail-closed yielded
  outcome rather than expecting the state-machine run itself to fail. Preserve
  zero authorization and zero execution assertions.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib agent::goose::tests`;
  `git diff --check`.

### G4-O — primary-turn capability integration (wave 4.6, lane integration)

- Depends: G4-E2, G4-F, G4-L, G4-MX.
- Change: `crates/openhuman-core/src/agent/harness/session/runtime/primary_turn.rs`;
  `crates/openhuman-core/src/web_chat/run_task.rs`.
- Context: `crates/openhuman-core/src/agent/harness/session/types.rs`;
  `crates/openhuman-core/src/agent/primary_orchestration/planner.rs`;
  `crates/openhuman-core/src/web_chat/types.rs`.
- Build the immutable plan before Goose, combine persisted permission with the
  downward turn override, pass ordered tools, return deterministic unavailable
  output without a model/tool call when appropriate, and log mode, ordered
  routes, monetary boundary, context, traffic, and stop reason without content.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib primary_turn::tests`;
  `git diff --check`.

### G4-P — Gate 4 integration fixtures (wave 4.7, lane acceptance)

- Depends: G4-NX, G4-O.
- Change: `crates/openhuman-core/src/agent/primary_orchestration/gate4_tests.rs`;
  `crates/openhuman-core/src/agent/primary_orchestration/mod.rs`.
- Context: `crates/openhuman-core/src/agent/primary_orchestration/planner.rs`;
  `crates/openhuman-core/src/agent/harness/session/runtime/primary_turn.rs`.
- Add network-free fixtures proving every Gate 4 requirement, including zero
  paid execution with auth present but opt-in absent and exact managed
  visibility only after persisted opt-in.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib gate4_tests`;
  `git diff --check`.

## Gate 5 — Qwen protocol adapter

### G5-A — Qwen normalization module (wave 5.0, lane protocol)

- Depends: Gate 4 passed.
- Change: `crates/openhuman-core/src/agent/goose/qwen.rs`;
  `crates/openhuman-core/src/agent/goose/mod.rs`.
- Context: `crates/openhuman-core/src/agent/goose/convert.rs`;
  `crates/openhuman-core/src/agent/tinyagents/model.rs`;
  `crates/openhuman-core/src/agent/goose/inference.rs`.
- Implement a pure complete-response normalizer modeled on pinned Qwen-Agent:
  native calls win; canonical `{name,arguments}` tags parse against advertised
  schemas; IDs/order survive; only the first action survives; result/exit
  boundaries stop parsing; reasoning stays separate; raw tags never become
  visible; destructive missing names/arguments are never inferred. Declare an
  external test module.
- Verify: `cargo check --manifest-path Cargo.toml -p openhuman --lib`;
  `git diff --check`.

### G5-B — official-style Qwen fixtures (wave 5.1, lane protocol)

- Depends: G5-A.
- Change: `crates/openhuman-core/src/agent/goose/qwen_tests.rs`.
- Context: `crates/openhuman-core/src/agent/goose/qwen.rs`;
  `crates/openhuman-core/src/agent/goose/convert.rs`.
- Cover final text, native precedence, canonical calls, complete assembled
  stream payload, multiple calls truncated to one, malformed/unknown/ambiguous
  calls, hidden reasoning, raw-tag stripping, result/exit boundaries, IDs, and
  missing destructive fields. No network.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib qwen_tests`;
  `git diff --check`.

### G5-AX — Qwen normalizer compile seam (wave 5.0, lane protocol-repair)

- Depends: G5-A.
- Change: `crates/openhuman-core/src/agent/goose/qwen.rs`.
- Context: none.
- Remove the unsupported `PartialEq` derive from `NormalizedQwenResponse`;
  `tinyinference::ModelResponse` does not implement that trait. Preserve behavior
  and every other derive.
- Verify: `cargo check --manifest-path Cargo.toml -p openhuman --lib`;
  `git diff --check`.

### G5-AY — Qwen stripped-markup whitespace seam (wave 5.1, lane protocol-repair)

- Depends: G5-B.
- Change: `crates/openhuman-core/src/agent/goose/qwen.rs`.
- Context: `crates/openhuman-core/src/agent/goose/qwen_tests.rs`.
- Assemble consecutive response text blocks without injecting separators and,
  when removing a complete tool-call block, avoid leaving a duplicate blank
  line between the visible prefix and suffix. Preserve ordinary text whitespace.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib qwen_tests`;
  `git diff --check`.

### G5-C — persisted protocol correction state (wave 5.0, lane persistence)

- Depends: Gate 4 passed.
- Change: `crates/openhuman-core/src/agent/goose/types.rs`;
  `crates/openhuman-core/src/agent/goose/store.rs`.
- Context: `crates/openhuman-core/src/agent/goose/tests.rs`.
- Persist correction count and terminal protocol failure in `GooseCheckpoint`;
  preserve them through replace-conversation, CAS, file reload, and serde
  defaults. Add effect handling without altering action/result pairing.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib agent::goose::tests`;
  `git diff --check`.

### G5-D — exact-route inference integration (wave 5.2, lane protocol)

- Depends: G5-B, G5-C.
- Change: `crates/openhuman-core/src/agent/goose/inference.rs`;
  `crates/openhuman-core/src/agent/goose/qwen.rs`.
- Context: `crates/openhuman-core/src/agent/goose/types.rs`;
  `crates/openhuman-core/src/agent/goose/runner.rs`.
- Apply Qwen normalization only when provider/model is the exact local binding.
  Assemble streaming output before normalization. On first invalid call append
  one schema-informed, non-user-visible correction and re-enter Goose; on the
  second, append one clear terminal answer. Other providers remain mechanical.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib qwen_tests`;
  `git diff --check`.

This combined lane was superseded after its isolated worker timed out with no
accepted patch. Execute G5-D1 then G5-D2 instead.

### G5-D1 — Qwen correction text helpers (wave 5.2, lane protocol-helper)

- Depends: G5-B, G5-C.
- Change: `crates/openhuman-core/src/agent/goose/qwen.rs`.
- Context: `crates/openhuman-core/src/agent/primary_orchestration/mode.rs`.
- Add pure exact-binding, schema-informed hidden-correction, and raw-markup-free
  terminal-failure text helpers. Do not echo invalid payloads or infer values.
- Verify: `cargo check --manifest-path Cargo.toml -p openhuman --lib`;
  `git diff --check`.

### G5-D2 — exact-route inference effect flow (wave 5.2, lane protocol-runtime)

- Depends: G5-D1.
- Change: `crates/openhuman-core/src/agent/goose/inference.rs`.
- Context: `crates/openhuman-core/src/agent/goose/qwen.rs`;
  `crates/openhuman-core/src/agent/goose/types.rs`;
  `crates/openhuman-core/src/agent/goose/runner.rs`.
- Normalize only the exact local binding after complete invocation. Persist one
  hidden schema-informed correction and re-enter Goose; on another invalid call
  persist one clear terminal answer. Preserve mechanical behavior elsewhere.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib qwen_tests`;
  `git diff --check`.

### G5-E — correction, pairing, and resume fixtures (wave 5.3, lane acceptance)

- Depends: G5-D2.
- Change: `crates/openhuman-core/src/agent/goose/tests.rs`;
  `crates/openhuman-core/src/agent/goose/qwen_tests.rs`.
- Context: `crates/openhuman-core/src/agent/goose/store.rs`;
  `crates/openhuman-core/src/agent/goose/runner.rs`.
- Prove one correction maximum across a fresh store handle, one action per
  inference, every accepted call has exactly one result, and interruption or
  resume creates no duplicate effect/orphan. Assert no raw markup in events or
  persisted OpenHuman messages.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib agent::goose`;
  `git diff --check`.

Execute this acceptance lane as the two disjoint atomic lanes G5-E1 and G5-E2.

### G5-E1 — Qwen protocol boundary fixtures (wave 5.3, lane acceptance-protocol)

- Depends: G5-D2.
- Change: `crates/openhuman-core/src/agent/goose/qwen_tests.rs`.
- Context: `crates/openhuman-core/src/agent/goose/qwen.rs`;
  `crates/openhuman-core/src/agent/goose/inference.rs`.
- Prove exact route matching, schema-only correction text, safe terminal text,
  one surviving action, and no raw markup in normalized visible output.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib qwen_tests`;
  `git diff --check`.

### G5-E2 — correction persistence and pairing fixtures (wave 5.3, lane acceptance-state)

- Depends: G5-D2.
- Change: `crates/openhuman-core/src/agent/goose/tests.rs`.
- Context: `crates/openhuman-core/src/agent/goose/store.rs`;
  `crates/openhuman-core/src/agent/goose/runner.rs`.
- Prove one correction maximum across a fresh file-store handle, a second
  invalid response terminates, every accepted action has one result, and resume
  creates no duplicate effect/orphan or raw persisted markup.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib agent::goose::tests`;
  `git diff --check`.

### G5-AZ — Qwen correction prompt raw markup seam (wave 5.3, lane protocol-repair)

- Depends: G5-E2.
- Change: `crates/openhuman-core/src/agent/goose/qwen.rs`.
- Context: `crates/openhuman-core/src/agent/goose/qwen_tests.rs`.
- Remove raw `<tool_call>` wrappers from `build_protocol_correction_prompt` while
  preserving canonical JSON `{"name": "<exact_tool_name>", "arguments": { ... }}`
  correction guidance and exact-route behavior so raw protocol markup never enters
  persisted conversation history.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib agent::goose`;
  `git diff --check`.

## Gate 6 — completion, guards, and use cases

### G6-A — typed completion contracts (wave 6.0, lane completion)

- Depends: Gate 5 passed.
- Change: `crates/openhuman-core/src/agent/primary_orchestration/completion.rs`;
  `crates/openhuman-core/src/agent/primary_orchestration/mod.rs`.
- Context: `crates/openhuman-core/src/agent/primary_orchestration/intent.rs`;
  `crates/openhuman-core/src/agent/goose/types.rs`.
- Define machine-checkable contracts and observations for chat, sourced web,
  validated image retrieval, artifact generation, repository mutation with
  verification, scheduling identifier/state, clarification, and approval.
  Successful informational tools are not completion. Declare external tests.
- Verify: `cargo check --manifest-path Cargo.toml -p openhuman --lib`;
  `git diff --check`.

### G6-B — completion contract tests (wave 6.1, lane completion)

- Depends: G6-A.
- Change: `crates/openhuman-core/src/agent/primary_orchestration/completion_tests.rs`.
- Context: `crates/openhuman-core/src/agent/primary_orchestration/completion.rs`.
- Cover every completion type, false-completion cases, and deterministic
  rendering only for bounded complete typed results.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib completion_tests`;
  `git diff --check`.

### G6-C — persisted loop-guard state (wave 6.0, lane persistence)

- Depends: Gate 5 passed.
- Change: `crates/openhuman-core/src/agent/goose/types.rs`;
  `crates/openhuman-core/src/agent/goose/store.rs`.
- Context: `crates/openhuman-core/src/agent/primary_orchestration/completion.rs`.
- Persist last call signature, repeated typed failure, no-progress count,
  unavailable routes, completion state, and terminal reason with serde
  defaults. Preserve state across CAS, reload, and conversation replacement.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib agent::goose::tests`;
  `git diff --check`.

### G6-D — Goose loop-guard operation (wave 6.2, lane runtime)

- Depends: G6-B, G6-C.
- Change: `crates/openhuman-core/src/agent/goose/runner.rs`;
  `crates/openhuman-core/src/agent/goose/tests.rs`.
- Context: `crates/openhuman-core/src/agent/goose/types.rs`;
  `crates/openhuman-core/src/agent/primary_orchestration/completion.rs`.
- Add typed Goose operations for duplicate signature, repeated failure,
  unavailable request, two no-progress passes, completion, and call ceiling.
  Agent ceiling yields resumable state; chat/assist terminate boundedly.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib agent::goose::tests`;
  `git diff --check`.

### G6-E — route failure and authorized fallback (wave 6.2, lane tools)

- Depends: G6-C, G4-M.
- Change: `crates/openhuman-core/src/agent/goose/tools.rs`;
  `crates/openhuman-core/src/agent/goose/types.rs`.
- Context: `crates/openhuman-core/src/agent/primary_orchestration/capability.rs`;
  `crates/openhuman-core/src/agent/primary_orchestration/planner.rs`.
- Classify balance/quota/credential/provider terminal failures, mark that route
  unavailable for the turn, and retain only already-authorized same-boundary
  alternatives. Never auto-execute a fallback or cross retrieval/generation,
  cost, permission, effect, or modality.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib agent::goose::tests`;
  `git diff --check`.

### G6-F — acceptance harness scaffold (wave 6.1, lane acceptance)

- Depends: G6-A.
- Change: `crates/openhuman-core/src/agent/phase7_acceptance_tests.rs`;
  `crates/openhuman-core/src/agent/mod.rs`.
- Context: `crates/openhuman-core/src/agent/harness/session/runtime/primary_turn.rs`;
  `crates/openhuman-core/src/agent/goose/runner.rs`.
- Add reusable recording model/tools, endpoint counter, effect counter, healthy
  and failed memory stubs, progress capture, and child-module declarations.
  All transports are in-process mocks.
- Verify: `cargo check --manifest-path Cargo.toml -p openhuman --tests`;
  `git diff --check`.

### G6-G — web and media use cases (wave 6.3, lane acceptance-web)

- Depends: G6-D, G6-E, G6-F.
- Change: `crates/openhuman-core/src/agent/phase7_acceptance_web_tests.rs`;
  `crates/openhuman-core/src/agent/phase7_acceptance_tests.rs`.
- Context: `crates/openhuman-core/src/agent/primary_orchestration/planner.rs`;
  `crates/openhuman-core/src/agent/goose/qwen.rs`.
- Mock current news, known URL fetch-first, image retrieval with validated
  source/rendered result, generation available/unavailable, disabled metered
  generation, zero-balance terminal result, and same-boundary free fallback.
  Assert endpoint counts, completion, route order, and no modality crossing.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib phase7_acceptance_web`;
  `git diff --check`.

### G6-H — memory, repository, and scheduling use cases (wave 6.3, lane acceptance-action)

- Depends: G6-D, G6-F.
- Change: `crates/openhuman-core/src/agent/phase7_acceptance_action_tests.rs`;
  `crates/openhuman-core/src/agent/phase7_acceptance_tests.rs`.
- Context: `crates/openhuman-core/src/agent/primary_orchestration/planner.rs`;
  `crates/openhuman-core/src/agent/goose/tools.rs`.
- Mock unrelated chat with failed memory, explicit failed recall, healthy
  recall/store, approved repository edit plus verification, durable schedule
  ID/status, and denied approval. Assert workspace/security policy remains the
  execution authority.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib phase7_acceptance_action`;
  `git diff --check`.

### G6-I — interruption and accounting use cases (wave 6.4, lane acceptance-state)

- Depends: G6-G, G6-H.
- Change: `crates/openhuman-core/src/agent/phase7_acceptance_state_tests.rs`;
  `crates/openhuman-core/src/agent/phase7_acceptance_tests.rs`.
- Context: `crates/openhuman-core/src/agent/goose/store.rs`;
  `crates/openhuman-core/src/agent/goose/types.rs`.
- Cover cancellation before/during execution, fresh-process resume, no duplicate
  effect/orphan, call ceilings, explicit stop reasons, latest context occupancy
  versus cumulative traffic, and bounded deterministic final rendering.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib phase7_acceptance_state`;
  `git diff --check`.

### G6-J — final completion integration (wave 6.5, lane integration)

- Depends: G6-D, G6-E, G6-I.
- Change: `crates/openhuman-core/src/agent/harness/session/runtime/primary_turn.rs`;
  `crates/openhuman-core/src/agent/goose/runner.rs`.
- Context: `crates/openhuman-core/src/agent/primary_orchestration/completion.rs`;
  `crates/openhuman-core/src/agent/goose/types.rs`.
- Thread the completion contract and typed terminal reasons through primary
  execution, persisted state, progress, transcript, and sanitized logs. Do not
  report success from a mere tool observation.
- Verify: `cargo test --manifest-path Cargo.toml -p openhuman --lib phase7_acceptance`;
  `git diff --check`.

## Gate 7 — local live matrix

### G7-A — reproducible live-matrix runner (wave 7.0, lane tooling)

- Depends: Gate 6 passed.
- Change: `scripts/debug/local-qwen-phase7-live.mjs`;
  `scripts/debug/local-qwen-phase7-live.test.mjs`.
- Context: `scripts/debug/live-local-provider-smoke.sh`;
  `scripts/mock-api/routes/llm.mjs`;
  `crates/openhuman-core/src/web_chat/schemas.rs`.
- Implement fresh-thread scenarios 1–12, explicit opt-in setup/restore,
  structured event/log capture, endpoint counters, context/traffic extraction,
  timeout/cancellation, and JSON/Markdown evidence. Default test mode uses a
  local mock server; live mode must be explicit and only targets loopback.
- Verify: `node --test scripts/debug/local-qwen-phase7-live.test.mjs`;
  `git diff --check`.

### G7-B — live execution and evidence (wave 7.1, SOL verification)

- Depends: G7-A.
- No product files changed by a worker. SOL runs the exact script against
  `http://127.0.0.1:1234/v1`, inspects every transcript/effect/endpoint count,
  and records the 12-scenario artifact. Required assertions: `131072`; no false
  user trimming; greetings one call/zero tools; retrieval never generation;
  disabled metered zero paid requests; no raw markup, duplicate effect, or
  orphan; explicit stop reason for every scenario.

## Gate 8 — MSVC release and delivery

### G8-A — Windows evidence helper (wave 8.0, lane tooling)

- Depends: Gate 7 passed.
- Change: `scripts/release/phase7-windows-evidence.ps1`;
  `scripts/release/phase7-windows-evidence.tests.ps1`.
- Context: `app/package.json`; `crates/openhuman-app/tauri.conf.json`;
  `scripts/ci-cancel-aware.sh`.
- Add a non-destructive helper that verifies MSVC host/target, runs the supplied
  build command, locates release `OpenHuman.exe`, NSIS `.exe`, and MSI, rejects
  MinGW/missing artifacts, and emits exact absolute path, byte size, SHA-256,
  commit, branch, and status. Test discovery/hashing with temporary fake files;
  tests do not build or contact a network.
- Verify: `powershell -NoProfile -File scripts/release/phase7-windows-evidence.tests.ps1`;
  `git diff --check`.

### G8-B — final MSVC verification (wave 8.1, SOL verification)

- Depends: G8-A.
- SOL runs focused Gate 1–6 tests, frontend tests if touched, formatting,
  changed-target Clippy, typecheck, Rust layout/architecture/license checks,
  then the release core and Tauri `nsis,msi` package build through
  `scripts/ci-cancel-aware.sh`. No worker broad suite is trusted as acceptance.

### G8-C — install smoke and evidence reconciliation (wave 8.2, SOL verification)

- Depends: G8-B.
- SOL verifies binary dependencies, smoke-tests the release executable and
  installers using the repository's Windows process, records hashes and live
  log excerpts, reviews the exact final diff/license attribution, updates the
  authoritative gate table, commits, proves a clean tree, pushes
  `fix/local-qwen-history`, and checks the remote SHA equals local HEAD.

## Integration rules

SOL prepares every worker from the exact accepted integration HEAD, never from
another unreviewed worker. Parallel wave results are reviewed independently and
integrated one at a time; a later result is rebased/regenerated if its base is
stale. SOL checks changed paths against the contract, inspects the diff, reruns
the named test, then runs the gate suite. One bounded repair is allowed for a
localized defect; otherwise the lane is abandoned and re-scoped. Gate status is
updated only after all tasks and literal gate checks pass.

No Gate 5 worker starts before Gate 4 is accepted; no Gate 6 worker before Gate
5; no live run before Gate 6; and no release build before the live gate. This
preserves the roadmap's ordered gates while still permitting parallel disjoint
lanes inside each gate.
