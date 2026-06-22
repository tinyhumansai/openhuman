# Test Coverage Matrix

Canonical mapping of every product feature to its test source(s). Drives gap-fill PRs (#967, #968, #969, #970, #971) under epic #773.

**Status legend**

| Symbol | Meaning                                                                 |
| ------ | ----------------------------------------------------------------------- |
| ✅     | Covered — at least one test asserts the behaviour                       |
| 🟡     | Partial — touched by a broader spec, no dedicated assertion             |
| ❌     | Missing — no test today                                                 |
| 🚫     | Not driver-automatable — manual smoke (release-cut checklist, see #971) |

**Layer abbreviations**

| Code | Layer                                                                                |
| ---- | ------------------------------------------------------------------------------------ |
| `RU` | Rust unit (`#[cfg(test)]` inside `src/`)                                             |
| `RI` | Rust integration (`tests/*.rs`)                                                      |
| `VU` | Vitest unit (`app/src/**/*.test.ts(x)`)                                              |
| `WD` | WDIO E2E (`app/test/e2e/specs/*.spec.ts`) — Linux `tauri-driver` + macOS Appium Mac2 |
| `MS` | Manual smoke (release-cut checklist)                                                 |

**Update contract** — when a PR adds, removes, or changes a feature leaf, the matrix row must be updated in the same PR. Tracking guard: see #965.

---

## 0. Application Lifecycle

### 0.1 Application Download

| ID    | Feature                      | Layer | Test path(s)                    | Status | Notes                                 |
| ----- | ---------------------------- | ----- | ------------------------------- | ------ | ------------------------------------- |
| 0.1.1 | Direct Download Access       | MS    | release-manual-smoke (see #971) | 🚫     | DMG hosting + version landing page    |
| 0.1.2 | Version Compatibility Check  | MS    | release-manual-smoke            | 🚫     | Driver cannot assert OS-version gates |
| 0.1.3 | Corrupted Installer Handling | MS    | release-manual-smoke            | 🚫     | Mutated DMG validation; manual repro  |

### 0.2 Installation & Launch

| ID    | Feature                         | Layer | Test path(s)         | Status | Notes                                    |
| ----- | ------------------------------- | ----- | -------------------- | ------ | ---------------------------------------- |
| 0.2.1 | DMG Installation Flow           | MS    | release-manual-smoke | 🚫     | OS-level Finder drag                     |
| 0.2.2 | Gatekeeper Validation           | MS    | release-manual-smoke | 🚫     | OS-level signature check                 |
| 0.2.3 | Code Signing Verification       | MS    | release-manual-smoke | 🚫     | `codesign --verify` capture in checklist |
| 0.2.4 | First Launch Permissions Prompt | MS    | release-manual-smoke | 🚫     | TCC prompts non-driver-automatable       |

### 0.3 Updates & Reinstallation

| ID    | Feature                       | Layer    | Test path(s)                                                                | Status | Notes                                                                           |
| ----- | ----------------------------- | -------- | --------------------------------------------------------------------------- | ------ | ------------------------------------------------------------------------------- |
| 0.3.1 | Auto Update Check             | RU+RI+MS | `src/openhuman/update/` (Rust unit), `tests/json_rpc_e2e.rs`, release smoke | 🟡     | Core check/update policy covered; desktop prompt + release upgrade still manual |
| 0.3.2 | Forced Update Handling        | MS       | release-manual-smoke                                                        | 🚫     | End-to-end gating verified at release                                           |
| 0.3.3 | Reinstall with Existing State | MS       | release-manual-smoke                                                        | 🚫     | Workspace persistence on reinstall                                              |
| 0.3.4 | Clean Uninstall               | MS       | release-manual-smoke                                                        | 🚫     | OS removal paths                                                                |

---

## 1. Authentication & Identity

### 1.1 Multi-Provider Authentication

| ID    | Feature           | Layer | Test path(s)                            | Status | Notes                                           |
| ----- | ----------------- | ----- | --------------------------------------- | ------ | ----------------------------------------------- |
| 1.1.1 | Google Login      | WD    | `app/test/e2e/specs/login-flow.spec.ts` | ✅     | Deep-link branch covered                        |
| 1.1.2 | GitHub Login      | WD    | `login-flow.spec.ts`                    | ✅     | Deep-link branch covered                        |
| 1.1.3 | Twitter (X) Login | WD    | `login-flow.spec.ts`                    | 🟡     | Generic OAuth path; assert provider tag in #968 |
| 1.1.4 | Discord Login     | WD    | `login-flow.spec.ts`                    | 🟡     | Same — discord branch unasserted                |

### 1.2 Account Management

| ID    | Feature                    | Layer | Test path(s)                                  | Status | Notes                                        |
| ----- | -------------------------- | ----- | --------------------------------------------- | ------ | -------------------------------------------- |
| 1.2.1 | Account Creation & Mapping | WD+RI | `login-flow.spec.ts`, `tests/json_rpc_e2e.rs` | ✅     |                                              |
| 1.2.2 | Multi-Provider Linking     | WD    | _missing_ — tracked #968                      | ❌     | Need spec linking 4 providers to one account |
| 1.2.3 | Duplicate Account Handling | WD    | _missing_ — tracked #968                      | ❌     | Collision UX path                            |

### 1.3 Session Management

| ID    | Feature                | Layer | Test path(s)                            | Status | Notes                     |
| ----- | ---------------------- | ----- | --------------------------------------- | ------ | ------------------------- |
| 1.3.1 | Token Issuance         | WD+RI | `login-flow.spec.ts`, `json_rpc_e2e.rs` | ✅     |                           |
| 1.3.2 | Session Persistence    | WD    | `logout-relogin-onboarding.spec.ts`     | ✅     |                           |
| 1.3.3 | Refresh Token Rotation | VU    | _missing_ — tracked #968                | ❌     | Slice-level refresh logic |

### 1.4 Logout & Revocation

| ID    | Feature            | Layer | Test path(s)                        | Status | Notes                              |
| ----- | ------------------ | ----- | ----------------------------------- | ------ | ---------------------------------- |
| 1.4.1 | Session Logout     | WD    | `logout-relogin-onboarding.spec.ts` | ✅     |                                    |
| 1.4.2 | Global Logout      | WD    | _missing_ — tracked #968            | ❌     | Multi-session invalidation         |
| 1.4.3 | Token Invalidation | WD    | _missing_ — tracked #968            | ❌     | Server-side revocation propagation |

---

## 2. Permissions & System Access

### 2.1 macOS Permissions

| ID    | Feature                     | Layer | Test path(s)         | Status | Notes               |
| ----- | --------------------------- | ----- | -------------------- | ------ | ------------------- |
| 2.1.1 | Accessibility Permission    | MS    | release-manual-smoke | 🚫     | TCC OS-level prompt |
| 2.1.2 | Input Monitoring Permission | MS    | release-manual-smoke | 🚫     | TCC OS-level prompt |
| 2.1.3 | Screen Recording Permission | MS    | release-manual-smoke | 🚫     | TCC OS-level prompt |
| 2.1.4 | Microphone Permission       | MS    | release-manual-smoke | 🚫     | TCC OS-level prompt |

### 2.2 Permission Lifecycle

| ID    | Feature                           | Layer | Test path(s)                   | Status | Notes                          |
| ----- | --------------------------------- | ----- | ------------------------------ | ------ | ------------------------------ |
| 2.2.1 | Permission Grant Flow             | RU    | `src/openhuman/accessibility/` | 🟡     | Core branch covered; UX manual |
| 2.2.2 | Permission Denial Handling        | RU    | `src/openhuman/accessibility/` | 🟡     | Same                           |
| 2.2.3 | Permission Re-Sync / Refresh      | WD    | _missing_ — tracked #968       | ❌     | App-restart re-sync            |
| 2.2.4 | Partial Permission State Handling | WD    | _missing_ — tracked #968       | ❌     | macOS-only spec                |

---

## 3. Local AI Runtime (Ollama + LM Studio)

### 3.1 Model Management

| ID    | Feature                               | Layer | Test path(s)                                                                                                                                                                                                                                  | Status | Notes                                                                                                                |
| ----- | ------------------------------------- | ----- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | -------------------------------------------------------------------------------------------------------------------- |
| 3.1.1 | Model Detection                       | RU+WD | `src/openhuman/local_ai/`, `local-model-runtime.spec.ts`                                                                                                                                                                                      | ✅     |                                                                                                                      |
| 3.1.2 | Model Download & Installation         | WD    | `local-model-runtime.spec.ts`                                                                                                                                                                                                                 | ✅     |                                                                                                                      |
| 3.1.3 | Model Version Handling                | RU    | `src/openhuman/local_ai/model_ids.rs`                                                                                                                                                                                                         | ✅     |                                                                                                                      |
| 3.1.4 | LM Studio Model Discovery             | RU+RI | `src/openhuman/local_ai/service/ollama_admin_tests.rs`, `tests/json_rpc_e2e.rs`                                                                                                                                                               | ✅     | Uses LM Studio's OpenAI-compatible `/v1/models` surface                                                              |
| 3.1.5 | Model Context-Window Requirement Gate | RU+VU | `src/openhuman/inference/local/model_requirements.rs`, `src/openhuman/inference/local/ollama.rs`, `src/openhuman/inference/local/service/ollama_admin_tests.rs`, `app/src/components/settings/panels/local-model/ModelStatusSection.test.tsx` | ✅     | Rejects Ollama models whose native context window is below the memory-layer minimum (`local_ai.model_context_check`) |

### 3.2 Runtime Execution

| ID    | Feature                            | Layer | Test path(s)                                                                    | Status | Notes                                                    |
| ----- | ---------------------------------- | ----- | ------------------------------------------------------------------------------- | ------ | -------------------------------------------------------- |
| 3.2.1 | Local Inference Execution          | WD    | `local-model-runtime.spec.ts`                                                   | ✅     |                                                          |
| 3.2.2 | Resource Handling (CPU/GPU/Memory) | RU    | `src/openhuman/local_ai/device.rs`                                              | 🟡     | Detection unit; runtime constraint manual                |
| 3.2.3 | Runtime Failure Handling           | RU+WD | `local-model-runtime.spec.ts`                                                   | ✅     |                                                          |
| 3.2.4 | LM Studio Chat Completions         | RU+RI | `src/openhuman/local_ai/service/public_infer_tests.rs`, `tests/json_rpc_e2e.rs` | ✅     | Covers prompt/chat success and non-success status errors |

### 3.3 Runtime Configuration

#### 3.3.1 RAM Allocation Control

| ID      | Feature                    | Layer | Test path(s)                                 | Status | Notes                               |
| ------- | -------------------------- | ----- | -------------------------------------------- | ------ | ----------------------------------- |
| 3.3.1.1 | RAM Limit Selection        | VU    | `app/src/components/settings/` (panel-level) | 🟡     | UI present; assertion shallow       |
| 3.3.1.2 | RAM Availability Detection | RU    | `src/openhuman/local_ai/device.rs`           | ✅     |                                     |
| 3.3.1.3 | Over-Allocation Prevention | RU    | `src/openhuman/local_ai/ops.rs`              | 🟡     | Guard exists; explicit test pending |
| 3.3.1.4 | Under-Allocation Handling  | RU    | `src/openhuman/local_ai/ops.rs`              | 🟡     | Same                                |

#### 3.3.2 Dynamic Resource Adjustment

| ID      | Feature                         | Layer | Test path(s) | Status | Notes              |
| ------- | ------------------------------- | ----- | ------------ | ------ | ------------------ |
| 3.3.2.1 | Runtime Scaling Based on Load   | RU    | _missing_    | ❌     | Track in follow-up |
| 3.3.2.2 | Model Switching Based on Memory | RU    | _missing_    | ❌     | Track in follow-up |

#### 3.3.3 Configuration Persistence

| ID      | Feature                        | Layer    | Test path(s)                                                                                               | Status | Notes                                                  |
| ------- | ------------------------------ | -------- | ---------------------------------------------------------------------------------------------------------- | ------ | ------------------------------------------------------ |
| 3.3.3.1 | Save RAM Settings              | VU       | _missing_                                                                                                  | ❌     | Settings slice                                         |
| 3.3.3.2 | Apply on Restart               | WD       | `local-model-runtime.spec.ts`                                                                              | 🟡     | Restart not exercised                                  |
| 3.3.3.3 | Reset to Default               | VU       | _missing_                                                                                                  | ❌     |                                                        |
| 3.3.3.4 | Provider Selection Persistence | RU+RI+VU | `src/openhuman/config/ops_tests.rs`, `tests/json_rpc_e2e.rs`, `app/src/utils/tauriCommands/config.test.ts` | ✅     | Covers `lm_studio` normalization and config round-trip |

---

## 4. Chat Interface (Core Interaction)

### 4.1 Chat Sessions

| ID    | Feature                | Layer | Test path(s)                                                     | Status | Notes                                 |
| ----- | ---------------------- | ----- | ---------------------------------------------------------------- | ------ | ------------------------------------- |
| 4.1.1 | Session Creation       | WD    | `conversations-web-channel-flow.spec.ts`                         | ✅     |                                       |
| 4.1.2 | Session Persistence    | WD    | `conversations-web-channel-flow.spec.ts`                         | ✅     |                                       |
| 4.1.3 | Multi-Session Handling | WD    | `agent-review.spec.ts`, `conversations-web-channel-flow.spec.ts` | 🟡     | No dedicated multi-thread switch test |

### 4.2 Messaging

| ID    | Feature                                                        | Layer | Test path(s)                                                                                                                                                    | Status | Notes                                                                                                                                                       |
| ----- | -------------------------------------------------------------- | ----- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 4.2.1 | User Message Handling                                          | WD+RI | `conversations-web-channel-flow.spec.ts`, `tests/json_rpc_e2e.rs`                                                                                               | ✅     |                                                                                                                                                             |
| 4.2.2 | AI Response Generation                                         | WD    | `agent-review.spec.ts`                                                                                                                                          | ✅     | Mock LLM                                                                                                                                                    |
| 4.2.3 | Streaming Responses                                            | RI    | `tests/json_rpc_e2e.rs`, `tests/agent_harness_e2e.rs`                                                                                                           | ✅     | `tests/agent_harness_e2e.rs` adds provider-level SSE tool-arg accumulation (chunked args reassembled + parsed) and engine-level delta forwarding (#3471)     |
| 4.2.4 | Parallel inference (cross-thread + within-thread forked turns) | RU+VU | `src/openhuman/channels/providers/web_tests.rs`, `app/src/store/__tests__/chatRuntimeSlice.test.ts`, `app/src/providers/__tests__/ChatRuntimeProvider.test.tsx` | 🟡     | Concurrent same-/cross-thread dispatch, cooperative `CancellationToken` teardown, and parallel-lane stream routing covered; dedicated WD E2E is a follow-up |

### 4.3 Tool Invocation

| ID    | Feature                       | Layer | Test path(s)                                                                                          | Status | Notes                                                                                                                                                        |
| ----- | ----------------------------- | ----- | ----------------------------------------------------------------------------------------------------- | ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 4.3.1 | Tool Trigger via Chat         | WD    | `skill-execution-flow.spec.ts`, `skill-multi-round.spec.ts`                                           | ✅     |                                                                                                                                                              |
| 4.3.2 | Permission-Based Execution    | RU+WD | `src/openhuman/tools/`, `skill-execution-flow.spec.ts`                                                | ✅     |                                                                                                                                                              |
| 4.3.3 | Tool Failure Handling         | WD    | `skill-execution-flow.spec.ts`                                                                        | ✅     |                                                                                                                                                              |
| 4.3.4 | Subagent Mascot Visualization | VU    | `app/src/features/human/SubMascotLayer.test.tsx`, `app/src/features/human/HumanPage.test.tsx`         | ✅     | Renders spawned/completed/failed subagent timeline rows as colored companion mascots with activity bubbles                                                   |
| 4.3.5 | Image Tool Contracts          | RU    | `src/openhuman/image/`                                                                                | ✅     | High-level `image_generation` / `view_image` schema, gating, serialization, prompt guidance, and contract e2e coverage for #2984                             |
| 4.3.6 | Background Monitor Tools      | RU+RI | `src/openhuman/monitor/`, `src/openhuman/tools/ops_tests.rs`, `tests/json_rpc_e2e.rs`                 | ✅     | First-class monitor domain covers command denial, line streaming, timeout, stop, bounded output, registry exposure, and JSON-RPC list/read surface for #3371 |
| 4.3.7 | Mascot Avatar Animation       | VU    | `app/src/features/human/Mascot/RiveMascot.test.tsx`, `app/src/features/human/Mascot/riveMaps.test.ts` | ✅     | Rive `MascotSM` state machine: face→pose mapping, Oculus→`visme_codes` viseme normalization, and idle random pose rotation for the `tiny_mascot.riv` upgrade |

### 4.4 Agent Harness Behaviors

End-to-end coverage of the agent harness via the web-chat RPC surface against an in-test scripted upstream (`tests/agent_harness_e2e.rs`) plus browser-level Redux/DOM assertions (`app/test/e2e/specs/agent-harness-behaviors.spec.ts`). Added for #3471.

| ID     | Feature                                 | Layer | Test path(s)                                                                                                                 | Status | Notes                                                                                                          |
| ------ | --------------------------------------- | ----- | --------------------------------------------------------------------------------------------------------------------------- | ------ | -------------------------------------------------------------------------------------------------------------- |
| 4.4.1  | Subagent Delegation                     | RI+WD | `tests/agent_harness_e2e.rs`, `app/test/e2e/specs/agent-harness-behaviors.spec.ts`                                           | ✅     | Orchestrator→researcher delegation asserts ≥3 upstream calls, distinct researcher persona, no "Unknown tool:"  |
| 4.4.2  | Subagent Clarification (input-required) | RI+WD | `tests/agent_harness_e2e.rs`, `app/test/e2e/specs/agent-harness-behaviors.spec.ts`, `chat-harness-subagent-continue.spec.ts` | ✅     | Subagent question surfaced + reply completes turn; full `continue_subagent` resume covered by the continue spec |
| 4.4.3  | Approval Gate — Approve                  | RI+WD | `tests/agent_harness_e2e.rs`, `app/test/e2e/specs/agent-harness-behaviors.spec.ts`                                           | ✅     | `approval_request` event → approve → tool executes (timeline `success`, file written)                          |
| 4.4.4  | Approval Gate — Deny                     | RI+WD | `tests/agent_harness_e2e.rs`, `app/test/e2e/specs/agent-harness-behaviors.spec.ts`                                           | ✅     | Deny → tool not executed (timeline `error`, file unchanged), agent explains gracefully                         |
| 4.4.5  | Approval Gate — TTL Timeout              | RI    | `tests/agent_harness_e2e.rs`                                                                                                 | ✅     | `OPENHUMAN_APPROVAL_TTL_SECS` (debug-only) → no decision → auto-deny                                            |
| 4.4.6  | Subagent + Approval Gate                | RI    | `tests/agent_harness_e2e.rs`                                                                                                 | ✅     | Approval fires inside subagent context; decision propagates; artifact content asserted                        |
| 4.4.7  | Multi-turn State Persistence            | RI    | `tests/agent_harness_e2e.rs`                                                                                                 | ✅     | Turn-2 upstream request carries turn-1 user + assistant messages                                              |
| 4.4.8  | Parallel Subagent Fan-out               | RI    | `tests/agent_harness_e2e.rs`                                                                                                 | ✅     | `spawn_parallel_agents` 2-task fan-out; both child results reach synthesis call                               |
| 4.4.9  | Max Iterations / Empty Response         | RI    | `tests/agent_harness_e2e.rs`                                                                                                 | ✅     | Tool-call limit surface + empty-completion graceful terminal (both skip Sentry)                               |
| 4.4.10 | Provider Error Retry                    | RI    | `tests/agent_harness_e2e.rs`                                                                                                 | ✅     | First upstream 500 retried by ReliableProvider; second succeeds                                               |
| 4.4.11 | Inference Phase Transitions             | WD    | `app/test/e2e/specs/agent-harness-behaviors.spec.ts`                                                                         | ✅     | Redux `inferenceStatusByThread` observes `subagent` phase then clears to idle                                 |
| 4.4.12 | Tool Timeline Completeness              | WD    | `app/test/e2e/specs/agent-harness-behaviors.spec.ts`                                                                         | ✅     | Timeline entries carry id/name/status/round; subagent row reaches `success`; rounds non-decreasing            |

---

## 5. Built-in Intelligence Skills

### 5.1 Screen Intelligence

| ID    | Feature            | Layer | Test path(s)                              | Status | Notes |
| ----- | ------------------ | ----- | ----------------------------------------- | ------ | ----- |
| 5.1.1 | Screen Capture     | RI    | `tests/screen_intelligence_vision_e2e.rs` | ✅     |       |
| 5.1.2 | Context Extraction | RI    | `tests/screen_intelligence_vision_e2e.rs` | ✅     |       |
| 5.1.3 | Memory Injection   | RI    | `tests/memory_graph_sync_e2e.rs`          | ✅     |       |

### 5.2 Text Autocomplete

| ID    | Feature                      | Layer | Test path(s)                                                                                                                      | Status | Notes                                                                           |
| ----- | ---------------------------- | ----- | --------------------------------------------------------------------------------------------------------------------------------- | ------ | ------------------------------------------------------------------------------- |
| 5.2.1 | Inline Suggestion Generation | MS+WD | `app/test/e2e/specs/autocomplete-flow.spec.ts` (settings surface only); release-manual-smoke for real inline-gen                  | 🟡     | Settings panel mounts; inline-gen requires macOS TCC grants — manual only       |
| 5.2.2 | Debounce Handling            | VU    | `app/src/features/autocomplete/__tests__/useAutocompleteSkillStatus.test.tsx` (status surface); core debounce timing is Rust-side | ✅     | Was ❌ — status branches now covered                                            |
| 5.2.3 | Acceptance Trigger           | MS    | release-manual-smoke (#971)                                                                                                       | 🟡     | Real keypress acceptance into a third-party text field — not driver-automatable |

### 5.3 Voice Intelligence

| ID    | Feature                   | Layer | Test path(s)                                                                                                                                                       | Status | Notes                                                                                                             |
| ----- | ------------------------- | ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------ | ----------------------------------------------------------------------------------------------------------------- |
| 5.3.1 | Voice Input Capture       | WD    | `voice-mode.spec.ts`                                                                                                                                               | ✅     |                                                                                                                   |
| 5.3.2 | Speech-to-Text Processing | WD    | `voice-mode.spec.ts`                                                                                                                                               | ✅     |                                                                                                                   |
| 5.3.3 | Voice Command Execution   | WD    | `voice-mode.spec.ts`                                                                                                                                               | ✅     |                                                                                                                   |
| 5.3.4 | Mascot Voice Selection    | VU    | `app/src/store/__tests__/mascotSlice.test.ts`, `app/src/components/settings/panels/__tests__/VoicePanel.test.tsx`, `app/src/features/human/useHumanMascot.test.ts` | ✅     | Slice validation + persist REHYDRATE, Settings picker UI (#1762), `synthesizeSpeech` voiceId override propagation |

### 5.4 Persona

| ID    | Feature                    | Layer | Test path(s)                                                                                     | Status | Notes                                                                                        |
| ----- | -------------------------- | ----- | ------------------------------------------------------------------------------------------------ | ------ | -------------------------------------------------------------------------------------------- |
| 5.4.1 | Persona Name & Description | VU    | `app/src/store/personaSlice.test.ts`, `app/src/components/settings/panels/PersonaPanel.test.tsx` | ✅     | Slice validation + persist REHYDRATE scrub; Settings identity fields persist on save (#2345) |
| 5.4.2 | SOUL.md Edit & Reset       | RU+VU | `src/openhuman/workspace/rpc.rs`, `app/src/components/settings/panels/PersonaPanel.test.tsx`     | ✅     | Core read/write/reset with allowlist + size cap; panel loads, saves, resets over RPC (#2345) |
| 5.4.3 | Persona Settings Surface   | VU    | `app/src/components/settings/panels/PersonaPanel.test.tsx`                                       | ✅     | Bundles identity + SOUL.md + link to Mascot avatar/voice (#2345)                             |

---

## 6. System Tools & Agent Capabilities

### 6.1 File System

| ID    | Feature                      | Layer | Test path(s)                                                                                           | Status | Notes                                                                |
| ----- | ---------------------------- | ----- | ------------------------------------------------------------------------------------------------------ | ------ | -------------------------------------------------------------------- |
| 6.1.1 | File Read Access             | RU+WD | `src/openhuman/tools/impl/filesystem/file_read.rs`, `app/test/e2e/specs/tool-filesystem-flow.spec.ts`  | ✅     | Was 🟡 — WDIO drives memory_read_file + asserts via Node fs          |
| 6.1.2 | File Write Access            | RU+WD | `src/openhuman/tools/impl/filesystem/file_write.rs`, `app/test/e2e/specs/tool-filesystem-flow.spec.ts` | ✅     | Was 🟡 — WDIO drives memory_write_file + asserts bytes match on disk |
| 6.1.3 | Path Restriction Enforcement | RU+WD | `src/openhuman/tools/impl/filesystem/file_read.rs`, `app/test/e2e/specs/tool-filesystem-flow.spec.ts`  | ✅     | Was 🟡 — WDIO asserts traversal + absolute-path denial envelope      |

### 6.2 Shell & Git

| ID    | Feature                      | Layer | Test path(s)                                                                                                    | Status | Notes                                                                                            |
| ----- | ---------------------------- | ----- | --------------------------------------------------------------------------------------------------------------- | ------ | ------------------------------------------------------------------------------------------------ |
| 6.2.1 | Shell Command Execution      | RU+WD | `src/openhuman/tools/impl/system/shell.rs`, `app/test/e2e/specs/tool-shell-git-flow.spec.ts`                    | ✅     | Was 🟡 — WDIO asserts agent runtime + `tools_agent` registry contract; full LLM path tracked #68 |
| 6.2.2 | Command Restriction Handling | RU+WD | `src/openhuman/security/policy_tests.rs`, `app/test/e2e/specs/tool-shell-git-flow.spec.ts`                      | ✅     | Was 🟡 — WDIO locks denial envelope shape `{ ok:false, error }` consumed by the React UI         |
| 6.2.3 | Git Read Operations          | RU+WD | `src/openhuman/tools/impl/filesystem/git_operations_tests.rs`, `app/test/e2e/specs/tool-shell-git-flow.spec.ts` | ✅     | Was 🟡 — WDIO seeds a fixture repo in OPENHUMAN_WORKSPACE and asserts read ops succeed           |
| 6.2.4 | Git Write Operations         | RU+WD | `src/openhuman/tools/impl/filesystem/git_operations_tests.rs`, `app/test/e2e/specs/tool-shell-git-flow.spec.ts` | ✅     | Was 🟡 — WDIO commits into the same fixture and asserts log advances                             |

### 6.3 Sub-agent Orchestration

| ID    | Feature                      | Layer | Test path(s)                                                                                                          | Status | Notes                                                                                                                                              |
| ----- | ---------------------------- | ----- | --------------------------------------------------------------------------------------------------------------------- | ------ | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| 6.3.1 | Steer a running sub-agent    | RU    | `src/openhuman/agent_orchestration/running_subagents.rs`, `src/openhuman/agent_orchestration/tools/steer_subagent.rs` | ✅     | `steer_subagent` injects a steer/collect message into a running async sub-agent's run-queue; registry enforces parent ownership + terminal guard.  |
| 6.3.2 | Wait for a sub-agent result  | RU    | `src/openhuman/agent_orchestration/running_subagents.rs`, `src/openhuman/agent_orchestration/tools/wait_subagent.rs`  | ✅     | `wait_subagent` blocks on the completion `watch` with a timeout; prunes terminal entries, leaves entries intact on timeout.                        |
| 6.3.3 | Steer lands in child history | RU    | `src/openhuman/agent/harness/subagent_runner/ops_tests.rs::run_queue_steer_lands_in_subagent_history`                 | ✅     | End-to-end: a queued steer is drained by the child `run_turn_engine` and appears as a `[User steering message]` user turn in the provider request. |
| 6.3.4 | Subconscious trigger pipeline (normalize → dedupe/rate → gate → queue) | RU+RI | `src/openhuman/subconscious_triggers/`, `tests/subconscious_triggers_e2e.rs`                                          | ✅     | Event→Trigger normalization for cron/user/composio/sub-agent, dedupe TTL + per-source rate limit, LLM gate over `agent::triage`, priority queue with overflow eviction. |
| 6.3.5 | Long-lived subconscious orchestrator session                          | RU    | `src/openhuman/subconscious/session.rs`, `src/openhuman/subconscious/user_thread.rs`                                  | ✅     | Persistent compressed session backed by a reserved thread; `notify_user` handoff to the user-facing thread; mode→autonomy config parity. |
| 6.3.6 | Multi-party human↔subconscious↔sub-agent conversation                  | RI    | `tests/subconscious_conversation_e2e.rs`                                                                              | ✅     | Scripted Gate/SessionExecutor seam drives delegate→sub-agent→merge, failure/retry, interleaving, dedupe, and rate-limit scenarios through the real orchestrator. |
| 6.3.7 | Full-stack trigger pipeline with mocked LLM                            | RI    | `tests/subconscious_fullstack_e2e.rs` (feature `e2e-test-support`)                                                    | ✅     | Real `GatePass`+`LongLivedSession`+`Agent`+sub-agent run against a provider-layer mock (no network); promote/drop, persistence, real `spawn_subagent`. |
| 6.3.8 | Subconscious Triggers debug/manage panel (Brain)                       | WD    | `app/test/playwright/specs/subconscious-triggers.spec.ts`                                                            | ✅     | Brain→Subconscious panel: renders disabled baseline + hint + reserved thread ids; enable toggle → Pipeline Enabled + event_driven + orchestrator running; disable; refresh re-fetches. |
| 6.3.9 | Vision sub-agent reads attached images | RU | `src/openhuman/agent_registry/agents/loader.rs::vision_agent_loads_on_vision_hint`, `src/openhuman/inference/provider/factory_tests.rs::vision_tier_is_vision_capable`, `src/openhuman/agent/harness/engine/core.rs::gate_tests`, `src/openhuman/agent/multimodal_tests.rs::extract_image_placeholders_pulls_att_tokens_in_order` | ✅ | Orchestrator (non-vision `chat-v1`) keeps the image as a placeholder, delegates to `vision_agent` on the `vision-v1` tier, which rehydrates the on-disk attachment and reads it. Engine gate prefers per-tier `current_model_vision`; turn placeholders forwarded into the sub-agent prompt. |

---

## 7. Web & Network Capabilities

### 7.1 Browser

| ID    | Feature            | Layer | Test path(s)                                                                                             | Status | Notes                                                                                               |
| ----- | ------------------ | ----- | -------------------------------------------------------------------------------------------------------- | ------ | --------------------------------------------------------------------------------------------------- |
| 7.1.1 | Open URL           | RU+WD | `src/openhuman/tools/impl/browser/browser_open_tests.rs`, `app/test/e2e/specs/tool-browser-flow.spec.ts` | ✅     | Was ❌ — WDIO asserts agent runtime + browser-bearing registry; mock backend captures HTTP shape    |
| 7.1.2 | Browser Automation | RU+WD | `src/openhuman/tools/impl/browser/browser_tests.rs`, `app/test/e2e/specs/tool-browser-flow.spec.ts`      | ✅     | Was ❌ — WDIO locks tools_agent wildcard scope (exposes the 22-action automation schema to the LLM) |

### 7.2 Network

| ID    | Feature                    | Layer | Test path(s)                                                                                                                                | Status | Notes                                                                        |
| ----- | -------------------------- | ----- | ------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ---------------------------------------------------------------------------- |
| 7.2.1 | HTTP / API Requests        | RU+WD | `service-connectivity-flow.spec.ts`                                                                                                         | ✅     |                                                                              |
| 7.2.2 | Web Search Execution       | WD    | `skill-execution-flow.spec.ts`                                                                                                              | 🟡     | Generic skill path                                                           |
| 7.2.3 | TinyFish Integration Tools | RU    | `src/openhuman/integrations/tinyfish_tests.rs`, `src/openhuman/tools/ops_tests.rs::all_tools_executes_tinyfish_family_against_fake_backend` | ✅     | Backend-proxied Search, Fetch, and Agent run tools covered with fake backend |

---

## 8. Memory System (Persistent AI Memory)

### 8.1 Memory Operations

| ID    | Feature       | Layer | Test path(s)                                                                   | Status | Notes  |
| ----- | ------------- | ----- | ------------------------------------------------------------------------------ | ------ | ------ |
| 8.1.1 | Store Memory  | RI+WD | `tests/memory_roundtrip_e2e.rs`, `app/test/e2e/specs/memory-roundtrip.spec.ts` | ✅     | Was ❌ |
| 8.1.2 | Recall Memory | RI+WD | same                                                                           | ✅     | Was ❌ |
| 8.1.3 | Forget Memory | RI+WD | same                                                                           | ✅     | Was ❌ |

### 8.2 Memory Handling

| ID    | Feature                    | Layer | Test path(s)                                                                                                                                                                                                                    | Status | Notes                                                                                               |
| ----- | -------------------------- | ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | --------------------------------------------------------------------------------------------------- |
| 8.2.1 | Context Injection          | RI    | `tests/autocomplete_memory_e2e.rs`                                                                                                                                                                                              | ✅     |                                                                                                     |
| 8.2.2 | Memory Consistency         | RI    | `tests/memory_graph_sync_e2e.rs`, `tests/worker_c_modules_e2e.rs`                                                                                                                                                               | ✅     | Worker C RPC E2E verifies memory-tree ingest is reflected by `memory_sync_status_list`              |
| 8.2.3 | Memory Scaling             | RU    | `src/openhuman/memory/ingestion_tests.rs`                                                                                                                                                                                       | 🟡     | Soak/scale benchmark not asserted                                                                   |
| 8.2.4 | Raw-archive sync reconcile | RU+RI | `src/openhuman/memory_sync/sources/rebuild.rs`, `src/openhuman/memory_sync/workspace/periodic.rs`, `tests/json_rpc_e2e.rs` (`json_rpc_memory_sources_reconcile_reports_pending_raw_files`), `tests/memory_sync_pipeline_e2e.rs` | ✅     | Coverage gate + incremental rebuild + workspace periodic scheduler + `memory_sources_reconcile` RPC |

### 8.3 Memory Retrieval Benchmarks

| ID    | Feature                              | Layer | Test path(s)                                                                                    | Status | Notes                                                                                         |
| ----- | ------------------------------------ | ----- | ----------------------------------------------------------------------------------------------- | ------ | --------------------------------------------------------------------------------------------- |
| 8.3.2 | Cross-Chat Entity Discoverability    | RU    | `src/openhuman/memory/tree/retrieval/benchmarks.rs::bench_cross_chat_entity_discoverable`       | ✅     | Verifies entity canonicalisation across multiple chats                                        |
| 8.3.3 | Citation Bundle Provenance           | RU    | `src/openhuman/memory/tree/retrieval/benchmarks.rs::bench_citation_bundle_provenance`           | ✅     | Verifies source_ref and tree_scope are populated in retrieval hits                            |
| 8.3.4 | Citation Fetch Leaves Hydration      | RU    | `src/openhuman/memory/tree/retrieval/benchmarks.rs::bench_citation_fetch_leaves_hydrates`       | ✅     | Verifies fetch_leaves returns content for exact chunk IDs                                     |
| 8.3.7 | Long-Source Exact Leaf Retrieval     | RU    | `src/openhuman/memory/tree/retrieval/benchmarks.rs::bench_long_source_retrieves_exact_leaf`     | 🟡     | Embedder required for seal + chunking; test runs in inert mode but assertions are conditional |
| 8.3.9 | Scale Ingest 20 Sources No Real Data | RU    | `src/openhuman/memory/tree/retrieval/benchmarks.rs::bench_scale_ingest_20_sources_no_real_data` | ✅     | Verifies retrieval correctness at scale with synthetic data                                   |

### 8.4 Explicit User Preferences (Two-Lane)

| ID    | Feature                                    | Layer | Test path(s)                                                                                                   | Status | Notes                                                                   |
| ----- | ------------------------------------------ | ----- | -------------------------------------------------------------------------------------------------------------- | ------ | ----------------------------------------------------------------------- |
| 8.4.1 | Save Preference (general / situational)    | RU    | `src/openhuman/agent/tools/save_preference_tests.rs`                                                           | ✅     | `save_preference` tool → `user_pref_{general,situational}`, topic-keyed |
| 8.4.2 | Lane A — Standing Prefs in System Prompt   | RU    | `src/openhuman/learning/prompt_sections.rs`, `src/openhuman/agent/harness/session/turn_tests.rs`               | ✅     | General prefs rendered into the system prompt at thread start           |
| 8.4.3 | Lane B — Situational Recall (vector-gated) | RU    | `src/openhuman/memory/store/unified/query_tests.rs::recall_relevant_by_vector_gates_on_similarity`             | ✅     | Per-turn; relevant query injects, unrelated suppresses                  |
| 8.4.4 | Same-Topic Contradiction (replace)         | RU    | `src/openhuman/agent/tools/save_preference_tests.rs::recategorising_moves_pref_between_namespaces`             | ✅     | `ON CONFLICT REPLACE`; a topic lives in exactly one scope               |
| 8.4.5 | Cross-Topic Contradiction Surfacing        | RU    | `src/openhuman/agent/tools/save_preference_tests.rs::save_surfaces_related_preference_for_contradiction_check` | ✅     | Related prefs surfaced in the tool result for the chat agent to resolve |
| 8.4.6 | vector_chunks Model-Signature Recall Guard | RU    | `src/openhuman/memory/store/unified/query_tests.rs::vector_recall_excludes_other_model_signature`              | ✅     | Excludes cross-model vectors; dim-guards legacy rows                    |

---

## 9. Automation Engine

### 9.1 Task Scheduling

| ID    | Feature       | Layer | Test path(s)             | Status | Notes |
| ----- | ------------- | ----- | ------------------------ | ------ | ----- |
| 9.1.1 | Task Creation | WD    | `cron-jobs-flow.spec.ts` | ✅     |       |
| 9.1.2 | Task Update   | WD    | `cron-jobs-flow.spec.ts` | ✅     |       |
| 9.1.3 | Task Deletion | WD    | `cron-jobs-flow.spec.ts` | ✅     |       |

### 9.2 Cron Jobs

| ID    | Feature                    | Layer | Test path(s)             | Status | Notes |
| ----- | -------------------------- | ----- | ------------------------ | ------ | ----- |
| 9.2.1 | Cron Expression Validation | RU    | `src/openhuman/cron/`    | ✅     |       |
| 9.2.2 | Recurring Execution        | WD+RI | `cron-jobs-flow.spec.ts` | ✅     |       |

### 9.3 Remote Execution

| ID    | Feature                 | Layer | Test path(s)             | Status | Notes                    |
| ----- | ----------------------- | ----- | ------------------------ | ------ | ------------------------ |
| 9.3.1 | Remote Agent Scheduling | RI    | `tests/json_rpc_e2e.rs`  | 🟡     | Coverage thin            |
| 9.3.2 | Execution Trigger       | WD    | `cron-jobs-flow.spec.ts` | ✅     |                          |
| 9.3.3 | Retry Handling          | RU    | `src/openhuman/cron/`    | 🟡     | Backoff branches partial |

---

## 10. Unified Messaging Hub

### 10.1 Integration Setup

| ID     | Feature             | Layer | Test path(s)                                                                                                                                                                          | Status | Notes                                                                                                                                                                                                                                                                                                                                              |
| ------ | ------------------- | ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 10.1.1 | Telegram Connection | WD    | `telegram-flow.spec.ts`                                                                                                                                                               | ✅     |                                                                                                                                                                                                                                                                                                                                                    |
| 10.1.2 | WhatsApp Connection | WD    | `app/test/e2e/specs/whatsapp-flow.spec.ts`                                                                                                                                            | ✅     | Was ❌                                                                                                                                                                                                                                                                                                                                             |
| 10.1.3 | Gmail Connection    | WD    | `gmail-flow.spec.ts`                                                                                                                                                                  | ✅     |                                                                                                                                                                                                                                                                                                                                                    |
| 10.1.4 | Slack Connection    | WD    | `app/test/e2e/specs/slack-flow.spec.ts`                                                                                                                                               | ✅     | Was ❌                                                                                                                                                                                                                                                                                                                                             |
| 10.1.5 | Yuanbao Connection  | RU    | `src/openhuman/channels/providers/yuanbao/`, `src/openhuman/channels/controllers/ops.rs::tests::connect_yuanbao_*`, `src/openhuman/channels/runtime/startup.rs::yuanbao_secret_tests` | 🟡     | New API-key channel for Tencent Yuanbao. RU covers sign-token preflight (valid/invalid creds, env-override cluster routing), credentials store hydration (incl. stale app_key guard), and WS reconnect/shutdown. No WDIO spec yet — connect-flow UI is rendered via the generic `ChannelSetupModal` already exercised by other channel flow specs. |

### 10.2 Authentication & Authorization

| ID     | Feature                               | Layer | Test path(s)                                                                  | Status | Notes                                                                     |
| ------ | ------------------------------------- | ----- | ----------------------------------------------------------------------------- | ------ | ------------------------------------------------------------------------- |
| 10.2.1 | OAuth / API Token Handling            | WD    | `skill-oauth.spec.ts`                                                         | ✅     |                                                                           |
| 10.2.2 | Scope Selection (Read/Write/Initiate) | WD    | `gmail-flow.spec.ts`, `skill-oauth.spec.ts`, `composio-triggers-flow.spec.ts` | 🟡     | Multi-scope matrix not exhaustive; Gmail trigger OAuth read scope covered |
| 10.2.3 | Token Storage & Encryption            | RU    | `src/openhuman/encryption/`, `src/openhuman/credentials/`                     | ✅     |                                                                           |

### 10.3 Message Sync & Ingestion

| ID     | Feature                   | Layer | Test path(s)                                                                                      | Status | Notes                                                                                                                                           |
| ------ | ------------------------- | ----- | ------------------------------------------------------------------------------------------------- | ------ | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| 10.3.1 | Incoming Message Sync     | RU+WD | `src/openhuman/channels/tests/`, `gmail-flow.spec.ts`                                             | ✅     |                                                                                                                                                 |
| 10.3.2 | Message Deduplication     | RU    | `src/openhuman/channels/tests/`                                                                   | ✅     |                                                                                                                                                 |
| 10.3.3 | WhatsApp Agent Retrieval  | RU    | `src/openhuman/whatsapp_data/tools/`, `tests/json_rpc_e2e.rs::whatsapp_data_agent_tools_e2e_1341` | ✅     | Three read-only agent tools wrap the local SQLite store; ingest stays internal-only. See [`docs/whatsapp-data-flow.md`](whatsapp-data-flow.md). |
| 10.3.4 | Real-Time vs Delayed Sync | RU    | `src/openhuman/channels/tests/runtime_dispatch.rs`                                                | ✅     |                                                                                                                                                 |

### 10.4 Messaging Operations

| ID     | Feature               | Layer | Test path(s)                                  | Status | Notes                                 |
| ------ | --------------------- | ----- | --------------------------------------------- | ------ | ------------------------------------- |
| 10.4.1 | Send Message          | WD+RI | `gmail-flow.spec.ts`, `telegram-flow.spec.ts` | ✅     |                                       |
| 10.4.2 | Reply to Thread       | WD    | `gmail-flow.spec.ts`                          | ✅     |                                       |
| 10.4.3 | Initiate Conversation | WD    | `gmail-flow.spec.ts`                          | 🟡     | Telegram/WhatsApp/Slack not exercised |
| 10.4.4 | Attachment Handling   | WD    | `gmail-flow.spec.ts`                          | 🟡     | Attachment branch shallow             |

### 10.5 Cross-Channel Behavior

| ID     | Feature                | Layer | Test path(s)                                              | Status | Notes                                                                               |
| ------ | ---------------------- | ----- | --------------------------------------------------------- | ------ | ----------------------------------------------------------------------------------- |
| 10.5.1 | Channel Isolation      | RU    | `src/openhuman/channels/tests/identity.rs`                | ✅     |                                                                                     |
| 10.5.2 | Unified Inbox Handling | WD+RI | `channels-smoke.spec.ts`, `tests/worker_c_modules_e2e.rs` | 🟡     | UI assertion shallow; RI covers config-only channel status after connect/disconnect |
| 10.5.3 | Context Preservation   | RU    | `src/openhuman/channels/tests/context.rs`                 | ✅     |                                                                                     |

### 10.6 Permission Enforcement

| ID     | Feature                     | Layer | Test path(s)                  | Status | Notes    |
| ------ | --------------------------- | ----- | ----------------------------- | ------ | -------- |
| 10.6.1 | Read Access Enforcement     | RU+WD | `auth-access-control.spec.ts` | ✅     |          |
| 10.6.2 | Write Access Enforcement    | RU+WD | `auth-access-control.spec.ts` | ✅     |          |
| 10.6.3 | Initiate Action Enforcement | RU    | `src/openhuman/channels/`     | 🟡     | E2E thin |

### 10.7 Disconnect & Re-Setup

| ID     | Feature                | Layer | Test path(s)                                          | Status | Notes                                                               |
| ------ | ---------------------- | ----- | ----------------------------------------------------- | ------ | ------------------------------------------------------------------- |
| 10.7.1 | Integration Disconnect | WD+RI | `gmail-flow.spec.ts`, `tests/worker_c_modules_e2e.rs` | ✅     | RI covers `channels_disconnect` clearing config-only iMessage state |
| 10.7.2 | Token Revocation       | RU    | `src/openhuman/credentials/`                          | ✅     |                                                                     |
| 10.7.3 | Re-Authorization Flow  | WD    | `skill-oauth.spec.ts`                                 | 🟡     | Re-auth post-revoke not asserted                                    |
| 10.7.4 | Permission Re-Sync     | WD    | _missing_ — tracked #968                              | ❌     |                                                                     |

---

## 11. Intelligence & Insights

### 11.1 Analysis Engine

| ID      | Feature                                | Layer | Test path(s)                                                                                                                                                                                           | Status | Notes                                                                                                                                                         |
| ------- | -------------------------------------- | ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 11.1.1  | Multi-Source Analysis                  | RI    | `tests/memory_graph_sync_e2e.rs`                                                                                                                                                                       | 🟡     | Frontend trigger untested                                                                                                                                     |
| 11.1.2  | Actionable Item Extraction             | VU    | `app/src/components/intelligence/__tests__/utils.test.ts`                                                                                                                                              | ✅     | Was ❌                                                                                                                                                        |
| 11.1.3  | Analyze Trigger                        | WD    | `app/test/e2e/specs/insights-dashboard.spec.ts` mounts the route; explicit analyze-handler invocation TBD                                                                                              | 🟡     | Route mounts and search/filter UI assert — full analyze trigger flow tracked as follow-up                                                                     |
| 11.1.4  | MCP server (stdio + HTTP)              | RU    | `src/openhuman/mcp_server/`                                                                                                                                                                            | ✅     | Stdio framing plus Streamable HTTP/SSE session lifecycle; `McpHttpClient` round-trip tests                                                                    |
| 11.1.5  | Global tool registry                   | RI    | `src/openhuman/tool_registry/`, `tests/json_rpc_e2e.rs`, `tests/domain_modules_e2e.rs`, `tests/worker_b_domain_e2e.rs`                                                                                 | ✅     | Read-only MCP/controller discovery with routes, schemas, version, allowed agents, and health                                                                  |
| 11.1.6  | SearXNG MCP search                     | RU    | `src/openhuman/integrations/searxng.rs`, `src/openhuman/mcp_server/tools.rs`, `src/openhuman/tools/schemas.rs`                                                                                         | ✅     | Self-hosted search config, normalized results, MCP argument validation, and mocked HTTP execution                                                             |
| 11.1.7  | Bundled prompt resources               | RU    | `src/openhuman/mcp_server/resources.rs`, `src/openhuman/mcp_server/protocol.rs`                                                                                                                        | ✅     | `resources/list` catalog + `resources/read` happy path, -32002 unknown URI, -32602 missing param, catalog-mirrors-BUILTINS parity test                        |
| 11.1.8  | Resource templates list                | RU    | `src/openhuman/mcp_server/resources.rs`, `src/openhuman/mcp_server/protocol.rs`                                                                                                                        | ✅     | `resources/templates/list` returns `{resourceTemplates: []}` (static catalog), tolerates unknown/cursor params                                                |
| 11.1.10 | MCP registry install→connect→tool_call | RI    | `tests/json_rpc_e2e.rs` (`mcp_clients_install_connect_tool_call_happy_path`), `tests/mcp_registry_e2e.rs`, `src/openhuman/mcp_registry/setup_ops.rs` (#3039)                                           | ✅     | HTTP-RPC happy path install→connect→tool_call→update_env against `test-mcp-stub`; transport-aware install (stdio + http_remote) via `build_install_transport` |
| 11.1.11 | MCP env reconfigure + registry creds   | RI/VU | `tests/json_rpc_e2e.rs` (`mcp_clients_registry_settings_roundtrip`), `src/openhuman/mcp_registry/registries/mcp_official.rs`, `app/src/components/channels/mcp/InstalledServerDetail.test.tsx` (#3039) | ✅     | `update_env` persist+reconnect; `registry_settings` get/set with secrets write-only (config-first, env-fallback); reconfigure form validation                 |
| 11.1.12 | MCP UI surface + setup-agent client    | VU    | `app/src/components/channels/mcp/InstallDialog.test.tsx`, `app/src/services/api/mcpClientsApi.test.ts`, `app/src/services/api/mcpSetupApi.test.ts` (#3039)                                             | ✅     | Skills `?tab=mcp` renders `McpServersTab` (not Coming Soon); auto-connect on install (best-effort); typed `mcpSetupApi` wrapper                               |
| 11.1.13 | MCP HTTP-remote auth (token / Bearer / OAuth) + redirect resolution | RU/VU | `src/openhuman/mcp_registry/connections.rs` (`build_http_auth*`, `resolve_final_url`), `src/openhuman/mcp_registry/oauth.rs` (PKCE/token/bundle/callback port), `app/src/components/channels/mcp/ConnectAuthModal.test.tsx` (#3495) | ✅ | Bearer/raw scheme + custom headers; redirect-final-URL resolved before auth; OAuth dynamic client registration + PKCE + refresh; tokens MERGED into stored env; credentials stored encrypted locally, never sent to backend |
| 11.1.14 | MCP "Help & configure" assistant | VU/RU | `app/src/components/channels/mcp/ConfigAssistantPanel.test.tsx`, `app/src/components/channels/mcp/ConfigHelpModal.test.tsx`, `src/openhuman/mcp_registry/ops.rs` (`invoke_config_assist_agent`) (#3495) | ✅ | Fixed server-specific prompt runs an agentic turn scoped to web_search_tool/web_fetch/curl only; markdown-rendered reply; per-MCP chat persisted while on the detail page |
| 11.1.15 | Agent uses connected MCP servers in chat | RU | `src/openhuman/agent_registry/agents/loader.rs` (`orchestrator_subagents_include_mcp_agent`, `mcp_agent_drives_connected_servers_without_install_or_shell`, `planner_has_readonly_mcp_discovery_not_execute`), `src/openhuman/agent_registry/agents/orchestrator/prompt.rs` (`connected_mcp_block_*`), `src/openhuman/agent/harness/session/turn_tests.rs` (`mcp_announcement_fires_once_for_new_server`), `src/openhuman/mcp_registry/{tools,connections}.rs` (#3495) | ✅ | `use_mcp_server` delegate → `mcp_agent` worker (discover→list→call); `mcp_registry_list_tools` read-only discovery; orchestrator `## Connected MCP Servers` prompt block + mid-session connect announcement on the user turn; planner read-only MCP discovery (no `tool_call`) |

<!-- 11.1.9 Vault Markdown Writes — removed: Knowledge Vaults dropped (vault domain + VaultPanel deleted). -->

### 11.2 Insights Dashboard

| ID     | Feature            | Layer | Test path(s)                 | Status | Notes  |
| ------ | ------------------ | ----- | ---------------------------- | ------ | ------ |
| 11.2.1 | Memory View        | WD    | `insights-dashboard.spec.ts` | ✅     | Was ❌ |
| 11.2.2 | Source Filtering   | WD    | `insights-dashboard.spec.ts` | ✅     | Was ❌ |
| 11.2.3 | Search & Retrieval | WD    | `insights-dashboard.spec.ts` | ✅     | Was ❌ |

---

## 12. Rewards & Progression

> Frontend-only domain — no Rust core counterpart. Confirmed during #970
> investigation: there is no `src/openhuman/rewards/` module and no Redux
> `rewardsSlice`; snapshot is fetched per-mount via
> `app/src/services/api/rewardsApi.ts` and held in `Rewards.tsx` component
> state. Backend ownership lives in `tinyhumansai/backend` (`/rewards/me`).

### 12.1 Role Unlocking

| ID     | Feature                  | Layer | Test path(s)                                                                                     | Status | Notes                                                |
| ------ | ------------------------ | ----- | ------------------------------------------------------------------------------------------------ | ------ | ---------------------------------------------------- |
| 12.1.1 | Activity-Based Unlock    | VU+WD | `app/src/store/__tests__/rewardsSlice.test.ts`, `app/test/e2e/specs/rewards-unlock-flow.spec.ts` | ✅     | Was ❌ — streak/feature-driven unlock branch         |
| 12.1.2 | Integration-Based Unlock | VU+WD | same                                                                                             | ✅     | Was ❌ — Discord membership → role assignment branch |
| 12.1.3 | Plan-Based Unlock        | VU+WD | same                                                                                             | ✅     | Was ❌ — plan tier + active subscription branch      |

### 12.2 Progress Tracking

| ID     | Feature                | Layer | Test path(s)                                                      | Status | Notes                                                                                               |
| ------ | ---------------------- | ----- | ----------------------------------------------------------------- | ------ | --------------------------------------------------------------------------------------------------- |
| 12.2.1 | Message Count Tracking | VU+WD | `rewardsSlice.test.ts`, `rewards-progression-persistence.spec.ts` | ✅     | Was ❌ — message-driven progress proxied by `metrics.featuresUsedCount` (no literal field)          |
| 12.2.2 | Usage Metrics          | VU+WD | same                                                              | ✅     | Was ❌ — current streak + cumulative tokens                                                         |
| 12.2.3 | State Persistence      | VU+WD | same                                                              | ✅     | Was ❌ — restart-equivalent (page unmount + remount + re-fetch); admin request log asserts re-fetch |

---

## 13. Settings & Developer Tools

### 13.1 Account & Security

| ID     | Feature                             | Layer      | Test path(s)                                                                                                                                                                | Status | Notes                                                                                                                                                                                                                                                                            |
| ------ | ----------------------------------- | ---------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 13.1.1 | Profile Management                  | VU         | `app/src/components/settings/panels/__tests__/PrivacyPanel.test.tsx`                                                                                                        | 🟡     |                                                                                                                                                                                                                                                                                  |
| 13.1.2 | Linked Accounts                     | WD         | `auth-access-control.spec.ts`                                                                                                                                               | 🟡     | UI surface unasserted                                                                                                                                                                                                                                                            |
| 13.1.3 | Meet Handoff Prompt-Injection Guard | VU         | `app/src/services/__tests__/webviewAccountService.meetPromptInjection.test.ts`                                                                                              | ✅     | Was ❌ — guard blocks handoff on hostile transcripts and wraps non-blocked transcripts in `<meeting_transcript source="untrusted_external_audio">` delimiters (#1920)                                                                                                            |
| 13.1.4 | Wallet Balances Panel               | VU         | `app/src/components/settings/panels/__tests__/WalletBalancesPanel.test.tsx`, `app/src/services/walletApi.test.ts`                                                           | ✅     | Loading/error/empty/loaded states; Retry + Refresh re-invocation; chain badges; truncated address; providerStatus chip                                                                                                                                                           |
| 13.1.5 | Approval History                    | VU         | `app/src/components/settings/panels/__tests__/ApprovalHistoryPanel.test.tsx`, `app/src/services/api/approvalApi.test.ts`                                                    | ✅     | Was ❌ — read-only audit surface over `approval_list_recent_decisions`; covers loaded/empty/error/refresh states, per-decision badge, and the bare-array vs `{result,logs}` envelope normalization                                                                               |
| 13.1.6 | Action Timeout                      | VU, RU, RI | `app/src/components/settings/panels/__tests__/AgentAccessPanel.test.tsx`, `src/openhuman/tool_timeout/mod.rs`, `src/openhuman/config/ops_tests.rs`, `tests/json_rpc_e2e.rs` | ✅     | New (#3100) — UI control over `config_get/update_agent_settings` for `[agent].agent_timeout_secs`; covers load/persist-on-blur/range-rejection/no-op/env-override-disable, the runtime-mutable `tool_timeout` resolver + env precedence, ops apply/reject, and the RPC roundtrip |

### 13.2 Automation & Channels

| ID     | Feature               | Layer | Test path(s)                                               | Status | Notes |
| ------ | --------------------- | ----- | ---------------------------------------------------------- | ------ | ----- |
| 13.2.1 | Channel Configuration | WD    | `app/test/e2e/specs/settings-channels-permissions.spec.ts` | ✅     |       |
| 13.2.2 | Permission Settings   | WD    | `app/test/e2e/specs/settings-channels-permissions.spec.ts` | ✅     |       |

### 13.3 AI & Skills

| ID     | Feature             | Layer | Test path(s)                                                                                                               | Status | Notes                   |
| ------ | ------------------- | ----- | -------------------------------------------------------------------------------------------------------------------------- | ------ | ----------------------- |
| 13.3.1 | Model Configuration | VU+WD | `app/src/components/settings/panels/__tests__/AutocompletePanel.test.tsx`, `app/test/e2e/specs/settings-ai-skills.spec.ts` | ✅     | AI-model-switch covered |
| 13.3.2 | Skill Toggle        | WD    | `skill-lifecycle.spec.ts`, `app/test/e2e/specs/settings-ai-skills.spec.ts`                                                 | ✅     |                         |

### 13.4 Developer Options

| ID     | Feature            | Layer | Test path(s)                                      | Status | Notes |
| ------ | ------------------ | ----- | ------------------------------------------------- | ------ | ----- |
| 13.4.1 | Webhook Inspection | WD    | `app/test/e2e/specs/settings-dev-options.spec.ts` | ✅     |       |
| 13.4.2 | Runtime Logs       | WD    | `app/test/e2e/specs/settings-dev-options.spec.ts` | ✅     |       |
| 13.4.3 | Memory Debug       | WD    | `app/test/e2e/specs/settings-dev-options.spec.ts` | ✅     |       |

### 13.5 Data Management

| ID     | Feature                                     | Layer | Test path(s)                                                                                                        | Status | Notes                                                                                                                                          |
| ------ | ------------------------------------------- | ----- | ------------------------------------------------------------------------------------------------------------------- | ------ | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| 13.5.1 | Clear App Data                              | WD    | `app/test/e2e/specs/settings-data-management.spec.ts`                                                               | ✅     | Destructive — confirm-then-reset                                                                                                               |
| 13.5.2 | Cache Reset                                 | WD    | `app/test/e2e/specs/settings-data-management.spec.ts`                                                               | ✅     |                                                                                                                                                |
| 13.5.3 | Full State Reset                            | WD    | `app/test/e2e/specs/settings-data-management.spec.ts`                                                               | ✅     | Restart-and-verify fresh-install state                                                                                                         |
| 13.5.4 | Migration from another assistant (OpenClaw) | VU+RU | `app/src/components/settings/panels/__tests__/MigrationPanel.test.tsx`, `src/openhuman/migration/ops.rs` (existing) | ✅     | Was ❌ — UI now wraps the existing `openhuman.migrate_openclaw` RPC with preview-then-apply + confirm. Hermes tracked as follow-up under #1440 |

---

## Summary

| Status           | Count                                            |
| ---------------- | ------------------------------------------------ |
| ✅ Covered       | 70                                               |
| 🟡 Partial       | 27                                               |
| ❌ Missing       | 26                                               |
| 🚫 Manual smoke  | 11                                               |
| **Total leaves** | **135 explicit + nested = 206 product features** |

PR-A delta: 13 leaves moved from ❌ → ✅ via 5 WDIO specs + 2 Vitest + 1 Rust integration test.
Remaining gaps tracked under sub-issues #965 (process), #966 (docs), #967 (tools), #968 (auth/perm), #969 (settings), #970 (rewards), #971 (manual smoke).
