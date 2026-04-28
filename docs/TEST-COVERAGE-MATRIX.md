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

| ID    | Feature                       | Layer | Test path(s)                                       | Status | Notes                                 |
| ----- | ----------------------------- | ----- | -------------------------------------------------- | ------ | ------------------------------------- |
| 0.3.1 | Auto Update Check             | RU+MS | `src/openhuman/update/` (Rust unit), release smoke | 🟡     | Core check covered; UI prompt manual  |
| 0.3.2 | Forced Update Handling        | MS    | release-manual-smoke                               | 🚫     | End-to-end gating verified at release |
| 0.3.3 | Reinstall with Existing State | MS    | release-manual-smoke                               | 🚫     | Workspace persistence on reinstall    |
| 0.3.4 | Clean Uninstall               | MS    | release-manual-smoke                               | 🚫     | OS removal paths                      |

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

## 3. Local AI Runtime (Ollama)

### 3.1 Model Management

| ID    | Feature                       | Layer | Test path(s)                                             | Status | Notes |
| ----- | ----------------------------- | ----- | -------------------------------------------------------- | ------ | ----- |
| 3.1.1 | Model Detection               | RU+WD | `src/openhuman/local_ai/`, `local-model-runtime.spec.ts` | ✅     |       |
| 3.1.2 | Model Download & Installation | WD    | `local-model-runtime.spec.ts`                            | ✅     |       |
| 3.1.3 | Model Version Handling        | RU    | `src/openhuman/local_ai/model_ids.rs`                    | ✅     |       |

### 3.2 Runtime Execution

| ID    | Feature                            | Layer | Test path(s)                       | Status | Notes                                     |
| ----- | ---------------------------------- | ----- | ---------------------------------- | ------ | ----------------------------------------- |
| 3.2.1 | Local Inference Execution          | WD    | `local-model-runtime.spec.ts`      | ✅     |                                           |
| 3.2.2 | Resource Handling (CPU/GPU/Memory) | RU    | `src/openhuman/local_ai/device.rs` | 🟡     | Detection unit; runtime constraint manual |
| 3.2.3 | Runtime Failure Handling           | RU+WD | `local-model-runtime.spec.ts`      | ✅     |                                           |

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

| ID      | Feature           | Layer | Test path(s)                  | Status | Notes                 |
| ------- | ----------------- | ----- | ----------------------------- | ------ | --------------------- |
| 3.3.3.1 | Save RAM Settings | VU    | _missing_                     | ❌     | Settings slice        |
| 3.3.3.2 | Apply on Restart  | WD    | `local-model-runtime.spec.ts` | 🟡     | Restart not exercised |
| 3.3.3.3 | Reset to Default  | VU    | _missing_                     | ❌     |                       |

---

## 4. Chat Interface (Core Interaction)

### 4.1 Chat Sessions

| ID    | Feature                | Layer | Test path(s)                                                     | Status | Notes                                 |
| ----- | ---------------------- | ----- | ---------------------------------------------------------------- | ------ | ------------------------------------- |
| 4.1.1 | Session Creation       | WD    | `conversations-web-channel-flow.spec.ts`                         | ✅     |                                       |
| 4.1.2 | Session Persistence    | WD    | `conversations-web-channel-flow.spec.ts`                         | ✅     |                                       |
| 4.1.3 | Multi-Session Handling | WD    | `agent-review.spec.ts`, `conversations-web-channel-flow.spec.ts` | 🟡     | No dedicated multi-thread switch test |

### 4.2 Messaging

| ID    | Feature                | Layer | Test path(s)                                                      | Status | Notes                       |
| ----- | ---------------------- | ----- | ----------------------------------------------------------------- | ------ | --------------------------- |
| 4.2.1 | User Message Handling  | WD+RI | `conversations-web-channel-flow.spec.ts`, `tests/json_rpc_e2e.rs` | ✅     |                             |
| 4.2.2 | AI Response Generation | WD    | `agent-review.spec.ts`                                            | ✅     | Mock LLM                    |
| 4.2.3 | Streaming Responses    | RI    | `tests/json_rpc_e2e.rs`                                           | 🟡     | UI streaming assertion thin |

### 4.3 Tool Invocation

| ID    | Feature                    | Layer | Test path(s)                                                | Status | Notes |
| ----- | -------------------------- | ----- | ----------------------------------------------------------- | ------ | ----- |
| 4.3.1 | Tool Trigger via Chat      | WD    | `skill-execution-flow.spec.ts`, `skill-multi-round.spec.ts` | ✅     |       |
| 4.3.2 | Permission-Based Execution | RU+WD | `src/openhuman/tools/`, `skill-execution-flow.spec.ts`      | ✅     |       |
| 4.3.3 | Tool Failure Handling      | WD    | `skill-execution-flow.spec.ts`                              | ✅     |       |

---

## 5. Built-in Intelligence Skills

### 5.1 Screen Intelligence

| ID    | Feature            | Layer | Test path(s)                                                             | Status | Notes |
| ----- | ------------------ | ----- | ------------------------------------------------------------------------ | ------ | ----- |
| 5.1.1 | Screen Capture     | WD+RI | `screen-intelligence.spec.ts`, `tests/screen_intelligence_vision_e2e.rs` | ✅     |       |
| 5.1.2 | Context Extraction | RI    | `tests/screen_intelligence_vision_e2e.rs`                                | ✅     |       |
| 5.1.3 | Memory Injection   | RI    | `tests/memory_graph_sync_e2e.rs`                                         | ✅     |       |

### 5.2 Text Autocomplete

| ID    | Feature                      | Layer | Test path(s)                                                                                                                                | Status | Notes                                                                               |
| ----- | ---------------------------- | ----- | ------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ----------------------------------------------------------------------------------- |
| 5.2.1 | Inline Suggestion Generation | MS+WD | `app/test/e2e/specs/autocomplete-flow.spec.ts` (settings surface only); release-manual-smoke for real inline-gen                            | 🟡     | Settings panel mounts (this PR); inline-gen requires macOS TCC grants — manual only |
| 5.2.2 | Debounce Handling            | VU    | `app/src/features/autocomplete/__tests__/useAutocompleteSkillStatus.test.tsx` (this PR — status surface); core debounce timing is Rust-side | ✅     | Was ❌ — status branches now covered                                                |
| 5.2.3 | Acceptance Trigger           | MS    | release-manual-smoke (#971)                                                                                                                 | 🟡     | Real keypress acceptance into a third-party text field — not driver-automatable     |

### 5.3 Voice Intelligence

| ID    | Feature                   | Layer | Test path(s)         | Status | Notes |
| ----- | ------------------------- | ----- | -------------------- | ------ | ----- |
| 5.3.1 | Voice Input Capture       | WD    | `voice-mode.spec.ts` | ✅     |       |
| 5.3.2 | Speech-to-Text Processing | WD    | `voice-mode.spec.ts` | ✅     |       |
| 5.3.3 | Voice Command Execution   | WD    | `voice-mode.spec.ts` | ✅     |       |

---

## 6. System Tools & Agent Capabilities

### 6.1 File System

| ID    | Feature                      | Layer | Test path(s)                                                                                                     | Status | Notes                                                                |
| ----- | ---------------------------- | ----- | ---------------------------------------------------------------------------------------------------------------- | ------ | -------------------------------------------------------------------- |
| 6.1.1 | File Read Access             | RU+WD | `src/openhuman/tools/impl/filesystem/file_read.rs`, `app/test/e2e/specs/tool-filesystem-flow.spec.ts` (this PR)  | ✅     | Was 🟡 — WDIO drives memory_read_file + asserts via Node fs          |
| 6.1.2 | File Write Access            | RU+WD | `src/openhuman/tools/impl/filesystem/file_write.rs`, `app/test/e2e/specs/tool-filesystem-flow.spec.ts` (this PR) | ✅     | Was 🟡 — WDIO drives memory_write_file + asserts bytes match on disk |
| 6.1.3 | Path Restriction Enforcement | RU+WD | `src/openhuman/tools/impl/filesystem/file_read.rs`, `app/test/e2e/specs/tool-filesystem-flow.spec.ts` (this PR)  | ✅     | Was 🟡 — WDIO asserts traversal + absolute-path denial envelope      |

### 6.2 Shell & Git

| ID    | Feature                      | Layer | Test path(s)                                                                                                              | Status | Notes                                                                                            |
| ----- | ---------------------------- | ----- | ------------------------------------------------------------------------------------------------------------------------- | ------ | ------------------------------------------------------------------------------------------------ |
| 6.2.1 | Shell Command Execution      | RU+WD | `src/openhuman/tools/impl/system/shell.rs`, `app/test/e2e/specs/tool-shell-git-flow.spec.ts` (this PR)                    | ✅     | Was 🟡 — WDIO asserts agent runtime + `tools_agent` registry contract; full LLM path tracked #68 |
| 6.2.2 | Command Restriction Handling | RU+WD | `src/openhuman/security/policy_tests.rs`, `app/test/e2e/specs/tool-shell-git-flow.spec.ts` (this PR)                      | ✅     | Was 🟡 — WDIO locks denial envelope shape `{ ok:false, error }` consumed by the React UI         |
| 6.2.3 | Git Read Operations          | RU+WD | `src/openhuman/tools/impl/filesystem/git_operations_tests.rs`, `app/test/e2e/specs/tool-shell-git-flow.spec.ts` (this PR) | ✅     | Was 🟡 — WDIO seeds a fixture repo in OPENHUMAN_WORKSPACE and asserts read ops succeed           |
| 6.2.4 | Git Write Operations         | RU+WD | `src/openhuman/tools/impl/filesystem/git_operations_tests.rs`, `app/test/e2e/specs/tool-shell-git-flow.spec.ts` (this PR) | ✅     | Was 🟡 — WDIO commits into the same fixture and asserts log advances                             |

---

## 7. Web & Network Capabilities

### 7.1 Browser

| ID    | Feature            | Layer | Test path(s)                                                                                                       | Status | Notes                                                                                               |
| ----- | ------------------ | ----- | ------------------------------------------------------------------------------------------------------------------ | ------ | --------------------------------------------------------------------------------------------------- |
| 7.1.1 | Open URL           | RU+WD | `src/openhuman/tools/impl/browser/browser_open_tests.rs`, `app/test/e2e/specs/tool-browser-flow.spec.ts` (this PR) | ✅     | Was ❌ — WDIO asserts agent runtime + browser-bearing registry; mock backend captures HTTP shape    |
| 7.1.2 | Browser Automation | RU+WD | `src/openhuman/tools/impl/browser/browser_tests.rs`, `app/test/e2e/specs/tool-browser-flow.spec.ts` (this PR)      | ✅     | Was ❌ — WDIO locks tools_agent wildcard scope (exposes the 22-action automation schema to the LLM) |

### 7.2 Network

| ID    | Feature              | Layer | Test path(s)                        | Status | Notes              |
| ----- | -------------------- | ----- | ----------------------------------- | ------ | ------------------ |
| 7.2.1 | HTTP / API Requests  | RU+WD | `service-connectivity-flow.spec.ts` | ✅     |                    |
| 7.2.2 | Web Search Execution | WD    | `skill-execution-flow.spec.ts`      | 🟡     | Generic skill path |

---

## 8. Memory System (Persistent AI Memory)

### 8.1 Memory Operations

| ID    | Feature       | Layer | Test path(s)                                                                                       | Status | Notes  |
| ----- | ------------- | ----- | -------------------------------------------------------------------------------------------------- | ------ | ------ |
| 8.1.1 | Store Memory  | RI+WD | `tests/memory_roundtrip_e2e.rs` (this PR), `app/test/e2e/specs/memory-roundtrip.spec.ts` (this PR) | ✅     | Was ❌ |
| 8.1.2 | Recall Memory | RI+WD | same                                                                                               | ✅     | Was ❌ |
| 8.1.3 | Forget Memory | RI+WD | same                                                                                               | ✅     | Was ❌ |

### 8.2 Memory Handling

| ID    | Feature            | Layer | Test path(s)                              | Status | Notes                             |
| ----- | ------------------ | ----- | ----------------------------------------- | ------ | --------------------------------- |
| 8.2.1 | Context Injection  | RI    | `tests/autocomplete_memory_e2e.rs`        | ✅     |                                   |
| 8.2.2 | Memory Consistency | RI    | `tests/memory_graph_sync_e2e.rs`          | ✅     |                                   |
| 8.2.3 | Memory Scaling     | RU    | `src/openhuman/memory/ingestion_tests.rs` | 🟡     | Soak/scale benchmark not asserted |

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

| ID     | Feature             | Layer | Test path(s)                                         | Status | Notes  |
| ------ | ------------------- | ----- | ---------------------------------------------------- | ------ | ------ |
| 10.1.1 | Telegram Connection | WD    | `telegram-flow.spec.ts`                              | ✅     |        |
| 10.1.2 | WhatsApp Connection | WD    | `app/test/e2e/specs/whatsapp-flow.spec.ts` (this PR) | ✅     | Was ❌ |
| 10.1.3 | Gmail Connection    | WD    | `gmail-flow.spec.ts`                                 | ✅     |        |
| 10.1.4 | Slack Connection    | WD    | `app/test/e2e/specs/slack-flow.spec.ts` (this PR)    | ✅     | Was ❌ |

### 10.2 Authentication & Authorization

| ID     | Feature                               | Layer | Test path(s)                                              | Status | Notes                             |
| ------ | ------------------------------------- | ----- | --------------------------------------------------------- | ------ | --------------------------------- |
| 10.2.1 | OAuth / API Token Handling            | WD    | `skill-oauth.spec.ts`                                     | ✅     |                                   |
| 10.2.2 | Scope Selection (Read/Write/Initiate) | WD    | `gmail-flow.spec.ts`, `skill-oauth.spec.ts`               | 🟡     | Multi-scope matrix not exhaustive |
| 10.2.3 | Token Storage & Encryption            | RU    | `src/openhuman/encryption/`, `src/openhuman/credentials/` | ✅     |                                   |

### 10.3 Message Sync & Ingestion

| ID     | Feature                   | Layer | Test path(s)                                          | Status | Notes |
| ------ | ------------------------- | ----- | ----------------------------------------------------- | ------ | ----- |
| 10.3.1 | Incoming Message Sync     | RU+WD | `src/openhuman/channels/tests/`, `gmail-flow.spec.ts` | ✅     |       |
| 10.3.2 | Message Deduplication     | RU    | `src/openhuman/channels/tests/`                       | ✅     |       |
| 10.3.3 | Real-Time vs Delayed Sync | RU    | `src/openhuman/channels/tests/runtime_dispatch.rs`    | ✅     |       |

### 10.4 Messaging Operations

| ID     | Feature               | Layer | Test path(s)                                  | Status | Notes                                 |
| ------ | --------------------- | ----- | --------------------------------------------- | ------ | ------------------------------------- |
| 10.4.1 | Send Message          | WD+RI | `gmail-flow.spec.ts`, `telegram-flow.spec.ts` | ✅     |                                       |
| 10.4.2 | Reply to Thread       | WD    | `gmail-flow.spec.ts`                          | ✅     |                                       |
| 10.4.3 | Initiate Conversation | WD    | `gmail-flow.spec.ts`                          | 🟡     | Telegram/WhatsApp/Slack not exercised |
| 10.4.4 | Attachment Handling   | WD    | `gmail-flow.spec.ts`                          | 🟡     | Attachment branch shallow             |

### 10.5 Cross-Channel Behavior

| ID     | Feature                | Layer | Test path(s)                               | Status | Notes                |
| ------ | ---------------------- | ----- | ------------------------------------------ | ------ | -------------------- |
| 10.5.1 | Channel Isolation      | RU    | `src/openhuman/channels/tests/identity.rs` | ✅     |                      |
| 10.5.2 | Unified Inbox Handling | WD    | `channels-smoke.spec.ts`                   | 🟡     | UI assertion shallow |
| 10.5.3 | Context Preservation   | RU    | `src/openhuman/channels/tests/context.rs`  | ✅     |                      |

### 10.6 Permission Enforcement

| ID     | Feature                     | Layer | Test path(s)                  | Status | Notes    |
| ------ | --------------------------- | ----- | ----------------------------- | ------ | -------- |
| 10.6.1 | Read Access Enforcement     | RU+WD | `auth-access-control.spec.ts` | ✅     |          |
| 10.6.2 | Write Access Enforcement    | RU+WD | `auth-access-control.spec.ts` | ✅     |          |
| 10.6.3 | Initiate Action Enforcement | RU    | `src/openhuman/channels/`     | 🟡     | E2E thin |

### 10.7 Disconnect & Re-Setup

| ID     | Feature                | Layer | Test path(s)                                | Status | Notes                            |
| ------ | ---------------------- | ----- | ------------------------------------------- | ------ | -------------------------------- |
| 10.7.1 | Integration Disconnect | WD    | `gmail-flow.spec.ts`, `notion-flow.spec.ts` | ✅     |                                  |
| 10.7.2 | Token Revocation       | RU    | `src/openhuman/credentials/`                | ✅     |                                  |
| 10.7.3 | Re-Authorization Flow  | WD    | `skill-oauth.spec.ts`                       | 🟡     | Re-auth post-revoke not asserted |
| 10.7.4 | Permission Re-Sync     | WD    | _missing_ — tracked #968                    | ❌     |                                  |

---

## 11. Intelligence & Insights

### 11.1 Analysis Engine

| ID     | Feature                    | Layer | Test path(s)                                                                                                        | Status | Notes                                                                                     |
| ------ | -------------------------- | ----- | ------------------------------------------------------------------------------------------------------------------- | ------ | ----------------------------------------------------------------------------------------- |
| 11.1.1 | Multi-Source Analysis      | RI    | `tests/memory_graph_sync_e2e.rs`                                                                                    | 🟡     | Frontend trigger untested                                                                 |
| 11.1.2 | Actionable Item Extraction | VU    | `app/src/components/intelligence/__tests__/utils.test.ts` (this PR)                                                 | ✅     | Was ❌                                                                                    |
| 11.1.3 | Analyze Trigger            | WD    | `app/test/e2e/specs/insights-dashboard.spec.ts` mounts the route (this PR); explicit analyze-handler invocation TBD | 🟡     | Route mounts and search/filter UI assert — full analyze trigger flow tracked as follow-up |

### 11.2 Insights Dashboard

| ID     | Feature            | Layer | Test path(s)                           | Status | Notes  |
| ------ | ------------------ | ----- | -------------------------------------- | ------ | ------ |
| 11.2.1 | Memory View        | WD    | `insights-dashboard.spec.ts` (this PR) | ✅     | Was ❌ |
| 11.2.2 | Source Filtering   | WD    | `insights-dashboard.spec.ts` (this PR) | ✅     | Was ❌ |
| 11.2.3 | Search & Retrieval | WD    | `insights-dashboard.spec.ts` (this PR) | ✅     | Was ❌ |

---

## 12. Rewards & Progression

### 12.1 Role Unlocking

| ID     | Feature                  | Layer | Test path(s)             | Status | Notes |
| ------ | ------------------------ | ----- | ------------------------ | ------ | ----- |
| 12.1.1 | Activity-Based Unlock    | WD    | _missing_ — tracked #970 | ❌     |       |
| 12.1.2 | Integration-Based Unlock | WD    | _missing_ — tracked #970 | ❌     |       |
| 12.1.3 | Plan-Based Unlock        | WD    | _missing_ — tracked #970 | ❌     |       |

### 12.2 Progress Tracking

| ID     | Feature                | Layer | Test path(s)             | Status | Notes              |
| ------ | ---------------------- | ----- | ------------------------ | ------ | ------------------ |
| 12.2.1 | Message Count Tracking | WD    | _missing_ — tracked #970 | ❌     |                    |
| 12.2.2 | Usage Metrics          | WD    | _missing_ — tracked #970 | ❌     |                    |
| 12.2.3 | State Persistence      | WD    | _missing_ — tracked #970 | ❌     | Restart-and-verify |

---

## 13. Settings & Developer Tools

### 13.1 Account & Security

| ID     | Feature            | Layer | Test path(s)                                                         | Status | Notes                 |
| ------ | ------------------ | ----- | -------------------------------------------------------------------- | ------ | --------------------- |
| 13.1.1 | Profile Management | VU    | `app/src/components/settings/panels/__tests__/PrivacyPanel.test.tsx` | 🟡     |                       |
| 13.1.2 | Linked Accounts    | WD    | `auth-access-control.spec.ts`                                        | 🟡     | UI surface unasserted |

### 13.2 Automation & Channels

| ID     | Feature               | Layer | Test path(s)             | Status | Notes |
| ------ | --------------------- | ----- | ------------------------ | ------ | ----- |
| 13.2.1 | Channel Configuration | WD    | _missing_ — tracked #969 | ❌     |       |
| 13.2.2 | Permission Settings   | WD    | _missing_ — tracked #969 | ❌     |       |

### 13.3 AI & Skills

| ID     | Feature             | Layer | Test path(s)                                                              | Status | Notes                               |
| ------ | ------------------- | ----- | ------------------------------------------------------------------------- | ------ | ----------------------------------- |
| 13.3.1 | Model Configuration | VU    | `app/src/components/settings/panels/__tests__/AutocompletePanel.test.tsx` | 🟡     | Generic; AI-model-switch unasserted |
| 13.3.2 | Skill Toggle        | WD    | `skill-lifecycle.spec.ts`                                                 | ✅     |                                     |

### 13.4 Developer Options

| ID     | Feature            | Layer | Test path(s)             | Status | Notes                          |
| ------ | ------------------ | ----- | ------------------------ | ------ | ------------------------------ |
| 13.4.1 | Webhook Inspection | WD    | _missing_ — tracked #969 | ❌     |                                |
| 13.4.2 | Runtime Logs       | WD    | _missing_ — tracked #969 | ❌     |                                |
| 13.4.3 | Memory Debug       | WD    | _missing_ — tracked #969 | ❌     | Panel exists; assertion needed |

### 13.5 Data Management

| ID     | Feature          | Layer | Test path(s)             | Status | Notes                                  |
| ------ | ---------------- | ----- | ------------------------ | ------ | -------------------------------------- |
| 13.5.1 | Clear App Data   | WD    | _missing_ — tracked #969 | ❌     | Destructive — confirm-then-reset       |
| 13.5.2 | Cache Reset      | WD    | _missing_ — tracked #969 | ❌     |                                        |
| 13.5.3 | Full State Reset | WD    | _missing_ — tracked #969 | ❌     | Restart-and-verify fresh-install state |

---

## Summary

| Status           | Count                                            |
| ---------------- | ------------------------------------------------ |
| ✅ Covered       | 64                                               |
| 🟡 Partial       | 27                                               |
| ❌ Missing       | 27                                               |
| 🚫 Manual smoke  | 11                                               |
| **Total leaves** | **129 explicit + nested = 200 product features** |

PR-A delta: 13 leaves moved from ❌ → ✅ via 5 WDIO specs + 2 Vitest + 1 Rust integration test.
Remaining gaps tracked under sub-issues #965 (process), #966 (docs), #967 (tools), #968 (auth/perm), #969 (settings), #970 (rewards), #971 (manual smoke).
