# Max Actions Per Hour UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the agent's `max_actions_per_hour` rate limit editable from Settings instead of requiring hand-edits to `config.toml`.

**Architecture:** Add a new `config_get_autonomy_settings` / `config_update_autonomy_settings` RPC pair (mirroring the existing `config_*_meet_settings` pair). Persist the new value to the user's `config.toml`. Surface it through a new `AutonomyPanel` linked from `DeveloperOptionsPanel`. Effect takes hold on the next `SecurityPolicy::from_config(...)` call (next chat / cron tick); running policies keep their existing limit — documented in helper text.

**Tech Stack:** Rust (`openhuman-core` lib, tokio, serde, schemars), TypeScript / React (`app/` workspace, Vite, Tailwind, Vitest, WDIO).

**Spec:** `docs/superpowers/specs/2026-05-22-max-actions-per-hour-ui-design.md`

**Branch:** `feat/ui-max-actions-per-hour` (already created; spec already committed).

**Note on scope refinement vs. spec**: the spec said "append an Agent autonomy subsection inside DeveloperOptionsPanel." On inspection, that panel is a list of `SettingsMenuItem` rows that each navigate to a dedicated subpanel; in-page form callouts (`CoreModeBadge`, `LogsFolderRow`, `SentryTestRow`) are reserved for tiny diagnostic widgets. A user-editable form belongs in its own subpanel — that also matches how every other autonomy/security knob added later would land. The plan therefore creates `AutonomyPanel.tsx` and adds a menu link in `DeveloperOptionsPanel`. Same UX intent, just plumbed via the standard pattern.

---

## File Structure

| File | Status | Responsibility |
| --- | --- | --- |
| `src/openhuman/config/ops.rs` | modify | Add `AutonomySettingsPatch` + `apply_autonomy_settings` + `load_and_apply_autonomy_settings` |
| `src/openhuman/config/ops_tests.rs` | modify | Unit tests for the new ops |
| `src/openhuman/config/schemas.rs` | modify | Add `AutonomySettingsUpdate` DTO, two `ControllerSchema` entries, two handlers, register both controllers |
| `src/openhuman/config/schemas_tests.rs` | modify | Handler-level tests through the controller registry |
| `tests/json_rpc_e2e.rs` | modify | New roundtrip test over real JSON-RPC |
| `app/src/services/rpcMethods.ts` | modify | Add two method-name constants |
| `app/src/utils/tauriCommands/config.ts` | modify | Add `openhumanGetAutonomySettings` + `openhumanUpdateAutonomySettings` wrappers |
| `app/src/utils/tauriCommands/config.test.ts` | modify | Unit tests for the two wrappers |
| `app/src/components/settings/panels/AutonomyPanel.tsx` | create | The form (number input + presets + save) |
| `app/src/components/settings/panels/__tests__/AutonomyPanel.test.tsx` | create | UI tests for the panel |
| `app/src/components/settings/panels/DeveloperOptionsPanel.tsx` | modify | Add menu item linking to the new panel |
| `app/src/components/settings/hooks/useSettingsNavigation.ts` | modify | Add `'autonomy'` to `SettingsRoute` union; add path detection |
| `app/src/pages/Settings.tsx` | modify | Register the `/settings/autonomy` route |
| `app/test/e2e/specs/settings-advanced-config.spec.ts` | modify | Add E2E case for save + persist |

No new top-level RPC namespaces, no schema-breaking changes to existing handlers, no changes to `SecurityPolicy` / `from_config` / consumers.

---

## Task 1: Rust — `AutonomySettingsPatch` + `apply_autonomy_settings`

**Files:**
- Modify: `src/openhuman/config/ops.rs` (add struct + function after existing `MeetSettingsPatch` at line 384 area)
- Test: `src/openhuman/config/ops_tests.rs`

- [ ] **Step 1: Write the failing test**

Append to `src/openhuman/config/ops_tests.rs` (after the existing `apply_meet_settings_updates_handoff_flag` test):

```rust
#[tokio::test]
async fn apply_autonomy_settings_persists_max_actions_per_hour() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let outcome = apply_autonomy_settings(
        &mut cfg,
        AutonomySettingsPatch {
            max_actions_per_hour: Some(200),
        },
    )
    .await
    .expect("apply");
    assert_eq!(cfg.autonomy.max_actions_per_hour, 200);
    // Snapshot returned so the caller can echo the saved state.
    assert!(outcome.value.get("config").is_some());
    // Round-trip from disk: reload the saved TOML and confirm.
    let on_disk = tokio::fs::read_to_string(&cfg.config_path).await.unwrap();
    assert!(
        on_disk.contains("max_actions_per_hour = 200"),
        "expected TOML to contain max_actions_per_hour = 200, got:\n{on_disk}"
    );
}

#[tokio::test]
async fn apply_autonomy_settings_no_op_when_patch_empty() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let prior = cfg.autonomy.max_actions_per_hour;
    let _ = apply_autonomy_settings(
        &mut cfg,
        AutonomySettingsPatch { max_actions_per_hour: None },
    )
    .await
    .expect("apply noop");
    assert_eq!(cfg.autonomy.max_actions_per_hour, prior);
}

#[tokio::test]
async fn apply_autonomy_settings_rejects_zero() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let err = apply_autonomy_settings(
        &mut cfg,
        AutonomySettingsPatch { max_actions_per_hour: Some(0) },
    )
    .await
    .unwrap_err();
    assert!(
        err.contains("between 1 and 10000"),
        "expected validation error, got: {err}"
    );
}

#[tokio::test]
async fn apply_autonomy_settings_rejects_above_cap() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let err = apply_autonomy_settings(
        &mut cfg,
        AutonomySettingsPatch { max_actions_per_hour: Some(10_001) },
    )
    .await
    .unwrap_err();
    assert!(err.contains("between 1 and 10000"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run from repo root:

```bash
pnpm debug rust apply_autonomy_settings
```

Expected: 4 tests fail to compile — `cannot find type AutonomySettingsPatch in this scope`, `cannot find function apply_autonomy_settings`. That's the failing state for TDD.

- [ ] **Step 3: Add the struct + function**

In `src/openhuman/config/ops.rs`, immediately after the existing `MeetSettingsPatch` definition (around line 386), add:

```rust
#[derive(Debug, Clone, Default)]
pub struct AutonomySettingsPatch {
    pub max_actions_per_hour: Option<u32>,
}
```

Then add the apply function. Put it next to `apply_meet_settings` (around line 764) so it's discoverable with the other settings ops:

```rust
/// Updates the autonomy policy settings in the configuration.
/// Validation: 1 <= max_actions_per_hour <= 10_000.
pub async fn apply_autonomy_settings(
    config: &mut Config,
    update: AutonomySettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    if let Some(v) = update.max_actions_per_hour {
        if v == 0 || v > 10_000 {
            return Err(format!(
                "max_actions_per_hour must be between 1 and 10000 (got {v})"
            ));
        }
        config.autonomy.max_actions_per_hour = v;
    }
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "autonomy settings saved to {}",
            config.config_path.display()
        )],
    ))
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
pnpm debug rust apply_autonomy_settings
```

Expected: all 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/openhuman/config/ops.rs src/openhuman/config/ops_tests.rs
git commit -m "feat(config): add AutonomySettingsPatch + apply_autonomy_settings"
```

---

## Task 2: Rust — `load_and_apply_autonomy_settings` roundtrip

**Files:**
- Modify: `src/openhuman/config/ops.rs` (add wrapper next to `load_and_apply_meet_settings` ~line 783)
- Test: `src/openhuman/config/ops_tests.rs`

- [ ] **Step 1: Write the failing test**

Append to `src/openhuman/config/ops_tests.rs`. Use the pattern from `load_and_apply_dictation_settings_rejects_invalid_activation_mode` at line 692 — that test shows how to set up `OPENHUMAN_WORKSPACE` so `load_config_with_timeout` reads the temp dir:

```rust
#[tokio::test]
async fn load_and_apply_autonomy_settings_roundtrip() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }

    let patch = AutonomySettingsPatch { max_actions_per_hour: Some(500) };
    let outcome = load_and_apply_autonomy_settings(patch).await.expect("apply");
    assert!(outcome.value.get("config").is_some());

    // Reload from scratch and confirm the saved value sticks.
    let reloaded = load_config_with_timeout().await.expect("reload");
    assert_eq!(reloaded.autonomy.max_actions_per_hour, 500);

    unsafe { std::env::remove_var("OPENHUMAN_WORKSPACE"); }
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
pnpm debug rust load_and_apply_autonomy_settings_roundtrip
```

Expected: fails — `cannot find function load_and_apply_autonomy_settings`.

- [ ] **Step 3: Add the wrapper**

In `src/openhuman/config/ops.rs`, immediately after `load_and_apply_meet_settings` (~line 783):

```rust
/// Loads the configuration, applies autonomy settings updates, and saves it.
pub async fn load_and_apply_autonomy_settings(
    update: AutonomySettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_autonomy_settings(&mut config, update).await
}
```

- [ ] **Step 4: Run to verify pass**

```bash
pnpm debug rust load_and_apply_autonomy_settings_roundtrip
```

Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add src/openhuman/config/ops.rs src/openhuman/config/ops_tests.rs
git commit -m "feat(config): add load_and_apply_autonomy_settings roundtrip"
```

---

## Task 3: Rust — `AutonomySettingsUpdate` DTO + schema entries

**Files:**
- Modify: `src/openhuman/config/schemas.rs`

This task is schema/registration plumbing — no test step yet. Tests come in Task 4 (handler) and Task 5 (E2E).

- [ ] **Step 1: Add the DTO**

In `src/openhuman/config/schemas.rs`, after the existing `MeetSettingsUpdate` struct (around line 120):

```rust
#[derive(Debug, Deserialize)]
struct AutonomySettingsUpdate {
    max_actions_per_hour: Option<u32>,
}
```

- [ ] **Step 2: Add schema definitions**

Inside the `schemas(name)` match block. Insert immediately after the `"get_meet_settings"` arm (around line 694):

```rust
        "update_autonomy_settings" => ControllerSchema {
            namespace: "config",
            function: "update_autonomy_settings",
            description:
                "Update agent autonomy policy settings (currently the per-hour tool action ceiling).",
            inputs: vec![FieldSchema {
                name: "max_actions_per_hour",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Maximum tool actions an agent may run per rolling hour (1-10000).",
                required: false,
            }],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "get_autonomy_settings" => ControllerSchema {
            namespace: "config",
            function: "get_autonomy_settings",
            description: "Read current agent autonomy policy settings.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "max_actions_per_hour",
                ty: TypeSchema::U64,
                comment: "Current maximum tool actions per rolling hour.",
                required: true,
            }],
        },
```

Note: `TypeSchema::U32` does not exist (see `src/core/mod.rs:81`). Use `U64` for the schema (informational); the DTO still uses `u32` and serde narrows the JSON number — out-of-range values get rejected by the validation in `apply_autonomy_settings`.

- [ ] **Step 3: Register in `all_controller_schemas`**

In the `all_controller_schemas()` vec (around line 207), append after `schemas("get_meet_settings")`:

```rust
        schemas("update_autonomy_settings"),
        schemas("get_autonomy_settings"),
```

- [ ] **Step 4: Verify it compiles**

```bash
cargo check --manifest-path Cargo.toml 2>&1 | tail -20
```

Expected: clean compile (or only an unused-function warning for the not-yet-wired handlers we'll add in Task 4).

- [ ] **Step 5: Commit**

```bash
git add src/openhuman/config/schemas.rs
git commit -m "feat(config): add autonomy_settings schemas + DTO"
```

---

## Task 4: Rust — handlers + controller registration

**Files:**
- Modify: `src/openhuman/config/schemas.rs`
- Test: `src/openhuman/config/schemas_tests.rs`

- [ ] **Step 1: Write the failing tests**

Look at `src/openhuman/config/schemas_tests.rs` for the testing convention used for other handlers — find an existing handler test (search for `handle_update_meet_settings` or `handle_get_meet_settings` in that file) and mirror the pattern. If no such test exists for meet, fall back to the analytics one (`handle_get_analytics_settings`). Append:

```rust
#[tokio::test]
async fn handle_get_autonomy_settings_returns_current_value() {
    let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    // Apply a known value first.
    let _ = crate::openhuman::config::ops::load_and_apply_autonomy_settings(
        crate::openhuman::config::ops::AutonomySettingsPatch {
            max_actions_per_hour: Some(123),
        },
    )
    .await
    .expect("seed");

    let out = super::handle_get_autonomy_settings(serde_json::Map::new())
        .await
        .expect("handler");
    let value = out.get("max_actions_per_hour").and_then(|v| v.as_u64());
    assert_eq!(value, Some(123));

    unsafe { std::env::remove_var("OPENHUMAN_WORKSPACE"); }
}

#[tokio::test]
async fn handle_update_autonomy_settings_rejects_invalid_value() {
    let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let mut params = serde_json::Map::new();
    params.insert("max_actions_per_hour".into(), serde_json::json!(0));

    let err = super::handle_update_autonomy_settings(params).await.unwrap_err();
    assert!(err.contains("between 1 and 10000"), "got: {err}");

    unsafe { std::env::remove_var("OPENHUMAN_WORKSPACE"); }
}
```

If `TEST_ENV_LOCK` isn't already imported at the top of `schemas_tests.rs`, mirror what `ops_tests.rs` does (`use crate::openhuman::config::TEST_ENV_LOCK;`). If `handle_get_autonomy_settings` / `handle_update_autonomy_settings` aren't visible (they're private fns in `schemas.rs`), use the controller-registry route shown in the alternative below.

**Alternative if private-fn access blocks compilation**: invoke through the registered controller dispatcher. Find an existing test in `schemas_tests.rs` that calls a controller by method name (`grep -n 'handle_' schemas_tests.rs`) and adapt it. The handler functions are `pub(super) fn handle_*` or `fn handle_*` — if they're not in scope, dispatching through `crate::core::dispatch::try_invoke_registered_rpc("openhuman.config_get_autonomy_settings", Map::new())` is the canonical alternative (this is what `src/core/all_tests.rs:436` does for `security_policy_info`).

- [ ] **Step 2: Run tests to verify they fail**

```bash
pnpm debug rust handle_get_autonomy_settings
```

Expected: fail — handlers don't exist / aren't registered.

- [ ] **Step 3: Add the handlers**

In `src/openhuman/config/schemas.rs`, immediately after `handle_get_meet_settings` (around line 1154-1176), add:

```rust
fn handle_update_autonomy_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[config][rpc] update_autonomy_settings enter");
        let update = match deserialize_params::<AutonomySettingsUpdate>(params) {
            Ok(u) => u,
            Err(err) => {
                log::warn!("[config][rpc] update_autonomy_settings invalid params: {err}");
                return Err(err);
            }
        };
        log::debug!(
            "[config][rpc] update_autonomy_settings patch max_actions_per_hour={:?}",
            update.max_actions_per_hour
        );
        let patch = config_rpc::AutonomySettingsPatch {
            max_actions_per_hour: update.max_actions_per_hour,
        };
        match config_rpc::load_and_apply_autonomy_settings(patch).await {
            Ok(outcome) => {
                log::debug!("[config][rpc] update_autonomy_settings ok");
                to_json(outcome)
            }
            Err(err) => {
                log::warn!("[config][rpc] update_autonomy_settings failed: {err}");
                Err(err)
            }
        }
    })
}

fn handle_get_autonomy_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        log::debug!("[config][rpc] get_autonomy_settings enter");
        let config = match config_rpc::load_config_with_timeout().await {
            Ok(c) => c,
            Err(err) => {
                log::warn!("[config][rpc] get_autonomy_settings load failed: {err}");
                return Err(err);
            }
        };
        let max_actions_per_hour = config.autonomy.max_actions_per_hour;
        log::debug!(
            "[config][rpc] get_autonomy_settings ok max_actions_per_hour={max_actions_per_hour}"
        );
        let result = serde_json::json!({
            "max_actions_per_hour": max_actions_per_hour,
        });
        to_json(RpcOutcome::new(
            result,
            vec!["autonomy settings read".to_string()],
        ))
    })
}
```

`config_rpc` here is the existing alias for `crate::openhuman::config::ops` — confirm by grepping (`grep -n 'config_rpc' src/openhuman/config/schemas.rs | head`).

- [ ] **Step 4: Register in `all_registered_controllers`**

In `src/openhuman/config/schemas.rs` `all_registered_controllers()` vec (around line 289-292), append after the `get_meet_settings` entry:

```rust
        RegisteredController {
            schema: schemas("update_autonomy_settings"),
            handler: handle_update_autonomy_settings,
        },
        RegisteredController {
            schema: schemas("get_autonomy_settings"),
            handler: handle_get_autonomy_settings,
        },
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
pnpm debug rust handle_get_autonomy_settings handle_update_autonomy_settings
```

Expected: both tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/openhuman/config/schemas.rs src/openhuman/config/schemas_tests.rs
git commit -m "feat(config): add autonomy_settings handlers + register controllers"
```

---

## Task 5: Rust — JSON-RPC E2E roundtrip

**Files:**
- Test: `tests/json_rpc_e2e.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/json_rpc_e2e.rs` (the file ends around line 3100; append after the last `#[tokio::test]`). Pattern adapted from the existing `json_rpc_web_chat_*` tests' setup:

```rust
#[tokio::test]
async fn json_rpc_config_autonomy_settings_roundtrip() {
    let _env_lock = json_rpc_e2e_env_lock();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config_with_local_ai_disabled(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // GET → expect the default (20).
    let initial = post_json_rpc(
        &rpc_base,
        7001,
        "openhuman.config_get_autonomy_settings",
        json!({}),
    )
    .await;
    let initial_result = assert_no_jsonrpc_error(&initial, "get_autonomy_settings initial");
    let initial_value = initial_result
        .get("result")
        .and_then(|r| r.get("max_actions_per_hour"))
        .and_then(Value::as_u64);
    assert_eq!(initial_value, Some(20), "expected default 20, got: {initial_result}");

    // UPDATE → 250.
    let update = post_json_rpc(
        &rpc_base,
        7002,
        "openhuman.config_update_autonomy_settings",
        json!({ "max_actions_per_hour": 250 }),
    )
    .await;
    assert_no_jsonrpc_error(&update, "update_autonomy_settings");

    // GET again → expect 250.
    let after = post_json_rpc(
        &rpc_base,
        7003,
        "openhuman.config_get_autonomy_settings",
        json!({}),
    )
    .await;
    let after_result = assert_no_jsonrpc_error(&after, "get_autonomy_settings after");
    let after_value = after_result
        .get("result")
        .and_then(|r| r.get("max_actions_per_hour"))
        .and_then(Value::as_u64);
    assert_eq!(after_value, Some(250));

    // Invalid value rejected.
    let bad = post_json_rpc(
        &rpc_base,
        7004,
        "openhuman.config_update_autonomy_settings",
        json!({ "max_actions_per_hour": 99999 }),
    )
    .await;
    let err = bad.get("error").cloned().unwrap_or_else(|| bad.clone());
    let err_str = err.to_string();
    assert!(
        err_str.contains("between 1 and 10000"),
        "expected validation error in: {err_str}"
    );

    mock_join.abort();
    rpc_join.abort();
}
```

- [ ] **Step 2: Run to verify it fails initially** (sanity — should fail if anything's mis-wired)

```bash
pnpm debug rust json_rpc_config_autonomy_settings_roundtrip
```

If Tasks 1-4 are all done correctly, this should already pass on first run. If it fails, follow the debug-log output and re-check the controller registration in Task 4 Step 4.

- [ ] **Step 3: Commit**

```bash
git add tests/json_rpc_e2e.rs
git commit -m "test(rpc): roundtrip for config_*_autonomy_settings"
```

---

## Task 6: TS — RPC method constants

**Files:**
- Modify: `app/src/services/rpcMethods.ts`

- [ ] **Step 1: Add constants**

In `app/src/services/rpcMethods.ts`, inside the `CORE_RPC_METHODS` object (keep alphabetical order — insert after `configGetAnalyticsSettings` at line 3):

```ts
  configGetAutonomySettings: 'openhuman.config_get_autonomy_settings',
```

and inside the update-settings group (after `configUpdateAnalyticsSettings` at line 7):

```ts
  configUpdateAutonomySettings: 'openhuman.config_update_autonomy_settings',
```

- [ ] **Step 2: Typecheck**

```bash
pnpm typecheck 2>&1 | tail -10
```

Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add app/src/services/rpcMethods.ts
git commit -m "feat(app): add autonomy_settings RPC method constants"
```

---

## Task 7: TS — wrapper functions

**Files:**
- Modify: `app/src/utils/tauriCommands/config.ts`

- [ ] **Step 1: Add the wrappers**

In `app/src/utils/tauriCommands/config.ts`, immediately after `openhumanGetMeetSettings` (around line 356, before the `ComposioTriggerSettingsUpdate` interface), add:

```ts
export async function openhumanUpdateAutonomySettings(update: {
  max_actions_per_hour?: number;
}): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: CORE_RPC_METHODS.configUpdateAutonomySettings,
    params: update,
  });
}

export async function openhumanGetAutonomySettings(): Promise<
  CommandResponse<{ max_actions_per_hour: number }>
> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<{ max_actions_per_hour: number }>>({
    method: CORE_RPC_METHODS.configGetAutonomySettings,
  });
}
```

- [ ] **Step 2: Typecheck**

```bash
pnpm typecheck 2>&1 | tail -10
```

Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add app/src/utils/tauriCommands/config.ts
git commit -m "feat(app): add openhuman{Get,Update}AutonomySettings wrappers"
```

---

## Task 8: TS — wrapper unit tests

**Files:**
- Test: `app/src/utils/tauriCommands/config.test.ts`

- [ ] **Step 1: Write the failing tests**

Append to `app/src/utils/tauriCommands/config.test.ts` (after the meet-settings describe blocks, around line 98). Pattern is the same as `openhumanUpdateMeetSettings` (lines 61-98):

```ts
  describe('openhumanUpdateAutonomySettings', () => {
    test('throws when not running in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(
        openhumanUpdateAutonomySettings({ max_actions_per_hour: 100 })
      ).rejects.toThrow('Not running in Tauri');
      expect(mockCallCoreRpc).not.toHaveBeenCalled();
    });

    test('forwards the patch to openhuman.config_update_autonomy_settings', async () => {
      mockCallCoreRpc.mockResolvedValue({
        result: { config: {}, workspace_dir: '/tmp', config_path: '/tmp/cfg.toml' },
        logs: [],
      });
      await openhumanUpdateAutonomySettings({ max_actions_per_hour: 100 });
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.config_update_autonomy_settings',
        params: { max_actions_per_hour: 100 },
      });
    });
  });

  describe('openhumanGetAutonomySettings', () => {
    test('throws when not running in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(openhumanGetAutonomySettings()).rejects.toThrow('Not running in Tauri');
      expect(mockCallCoreRpc).not.toHaveBeenCalled();
    });

    test('reads via openhuman.config_get_autonomy_settings', async () => {
      mockCallCoreRpc.mockResolvedValue({
        result: { max_actions_per_hour: 250 },
        logs: [],
      });
      const out = await openhumanGetAutonomySettings();
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.config_get_autonomy_settings',
      });
      expect(out.result.max_actions_per_hour).toBe(250);
    });
  });
```

Add the imports at the top of the file (find the existing `openhumanUpdateMeetSettings` import and add the new ones alongside it):

```ts
import {
  // ... existing imports ...
  openhumanGetAutonomySettings,
  openhumanUpdateAutonomySettings,
} from './config';
```

- [ ] **Step 2: Run to verify they pass**

```bash
pnpm debug unit app/src/utils/tauriCommands/config.test.ts -t "AutonomySettings"
```

Expected: 4 tests pass.

- [ ] **Step 3: Commit**

```bash
git add app/src/utils/tauriCommands/config.test.ts
git commit -m "test(app): cover openhuman{Get,Update}AutonomySettings wrappers"
```

---

## Task 9: New `AutonomyPanel.tsx`

**Files:**
- Create: `app/src/components/settings/panels/AutonomyPanel.tsx`

- [ ] **Step 1: Create the panel**

Write `app/src/components/settings/panels/AutonomyPanel.tsx`:

```tsx
import { useEffect, useState } from 'react';

import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import {
  openhumanGetAutonomySettings,
  openhumanUpdateAutonomySettings,
} from '../../../utils/tauriCommands/config';

const PRESETS = [
  { label: '20 (default)', value: 20 },
  { label: '100', value: 100 },
  { label: '500', value: 500 },
  { label: '1000', value: 1000 },
];

const MIN = 1;
const MAX = 10_000;

type Status =
  | { kind: 'idle' }
  | { kind: 'loading' }
  | { kind: 'saving' }
  | { kind: 'saved' }
  | { kind: 'error'; message: string };

const AutonomyPanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const [committed, setCommitted] = useState<number | null>(null);
  const [draft, setDraft] = useState<string>('');
  const [status, setStatus] = useState<Status>({ kind: 'loading' });

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const res = await openhumanGetAutonomySettings();
        if (cancelled) return;
        const value = res.result.max_actions_per_hour;
        setCommitted(value);
        setDraft(String(value));
        setStatus({ kind: 'idle' });
      } catch (err) {
        if (cancelled) return;
        setStatus({
          kind: 'error',
          message: err instanceof Error ? err.message : String(err),
        });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const parsed = Number.parseInt(draft, 10);
  const isValid =
    Number.isInteger(parsed) && parsed >= MIN && parsed <= MAX;
  const isChanged = committed !== null && parsed !== committed;
  const canSave = isValid && isChanged && status.kind !== 'saving';

  const applyPreset = (value: number) => {
    setDraft(String(value));
    if (status.kind === 'saved' || status.kind === 'error') {
      setStatus({ kind: 'idle' });
    }
  };

  const onSave = async () => {
    if (!canSave) return;
    setStatus({ kind: 'saving' });
    try {
      await openhumanUpdateAutonomySettings({ max_actions_per_hour: parsed });
      setCommitted(parsed);
      setStatus({ kind: 'saved' });
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      // Revert UI to last committed value, then surface the error.
      if (committed !== null) setDraft(String(committed));
      setStatus({ kind: 'error', message });
    }
  };

  return (
    <div className="z-10 relative">
      <SettingsHeader
        title="Agent autonomy"
        showBackButton
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />
      <div className="p-4 flex flex-col gap-4">
        <section className="px-4 py-3 rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900">
          <label
            htmlFor="autonomy-max-actions"
            className="block text-sm font-semibold text-stone-900 dark:text-neutral-100">
            Max actions per hour
          </label>
          <p className="text-xs text-stone-600 dark:text-neutral-400 mt-1">
            Maximum tool actions an agent can run per rolling hour. New value
            applies to your next chat — running sessions keep their current
            limit.
          </p>

          <div className="mt-3 flex items-center gap-2">
            <input
              id="autonomy-max-actions"
              type="number"
              min={MIN}
              max={MAX}
              step={1}
              value={draft}
              onChange={e => {
                setDraft(e.target.value);
                if (status.kind === 'saved' || status.kind === 'error') {
                  setStatus({ kind: 'idle' });
                }
              }}
              disabled={status.kind === 'loading' || status.kind === 'saving'}
              className="w-32 px-3 py-1.5 rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 text-sm font-mono"
            />
            <button
              onClick={onSave}
              disabled={!canSave}
              className="px-3 py-1.5 rounded-md bg-primary-600 hover:bg-primary-500 disabled:opacity-50 text-white text-xs font-medium transition-colors">
              {status.kind === 'saving' ? 'Saving…' : 'Save'}
            </button>
          </div>

          <div className="mt-3 flex flex-wrap gap-2">
            {PRESETS.map(p => (
              <button
                key={p.value}
                onClick={() => applyPreset(p.value)}
                className="px-2 py-1 rounded-md border border-stone-200 dark:border-neutral-800 text-xs text-stone-700 dark:text-neutral-200 hover:bg-stone-100 dark:hover:bg-neutral-800">
                {p.label}
              </button>
            ))}
          </div>

          <div
            role="status"
            aria-live="polite"
            aria-atomic="true"
            className="mt-3 text-xs min-h-[1rem]">
            {!isValid && draft.trim() !== '' && (
              <span className="text-coral-600 dark:text-coral-300">
                Must be an integer between {MIN} and {MAX.toLocaleString()}.
              </span>
            )}
            {status.kind === 'saved' && (
              <span className="text-sage-700 dark:text-sage-300">Saved.</span>
            )}
            {status.kind === 'error' && (
              <span className="text-coral-600 dark:text-coral-300">
                Failed: {status.message}
              </span>
            )}
          </div>
        </section>
      </div>
    </div>
  );
};

export default AutonomyPanel;
```

- [ ] **Step 2: Typecheck**

```bash
pnpm typecheck 2>&1 | tail -10
```

Expected: clean. If `useT` or i18n keys are missing for "Agent autonomy" / helper text, that's fine — strings are inline for now (i18n can be added later; matches the inline-string style used in `SentryTestRow`).

- [ ] **Step 3: Commit**

```bash
git add app/src/components/settings/panels/AutonomyPanel.tsx
git commit -m "feat(app): add AutonomyPanel for max_actions_per_hour control"
```

---

## Task 10: Wire `AutonomyPanel` into routing + Developer Options menu

**Files:**
- Modify: `app/src/components/settings/hooks/useSettingsNavigation.ts`
- Modify: `app/src/pages/Settings.tsx`
- Modify: `app/src/components/settings/panels/DeveloperOptionsPanel.tsx`

- [ ] **Step 1: Extend `SettingsRoute` union**

In `app/src/components/settings/hooks/useSettingsNavigation.ts`, find the `SettingsRoute` type (top of file, around line 5-40). Add `'autonomy'` to the union. Pick a logical spot — e.g. right after `'developer-options'`.

```ts
export type SettingsRoute =
  // ... existing variants ...
  | 'developer-options'
  | 'autonomy'
  // ... rest ...
```

Then add path detection in `getCurrentRoute()` (around line 94). Place it next to `'developer-options'`:

```ts
    if (path.includes('/settings/autonomy')) return 'autonomy';
```

- [ ] **Step 2: Register the route**

In `app/src/pages/Settings.tsx`:

1. Add import (alphabetical-ish with the other panel imports near the top):

```ts
import AutonomyPanel from '../components/settings/panels/AutonomyPanel';
```

2. Inside the `<Routes>` block (around line 355 next to the `developer-options` route), add:

```tsx
        <Route path="autonomy" element={wrapSettingsPage(<AutonomyPanel />)} />
```

- [ ] **Step 3: Add menu link in `DeveloperOptionsPanel`**

In `app/src/components/settings/panels/DeveloperOptionsPanel.tsx`, append to the `developerItems` array (after the `mcp-server` entry at line 240-256, before the closing `];`):

```tsx
  {
    id: 'autonomy',
    titleKey: 'settings.developerMenu.autonomy.title',
    descriptionKey: 'settings.developerMenu.autonomy.desc',
    route: 'autonomy',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z"
        />
      </svg>
    ),
  },
```

(SVG path is the standard "padlock" icon — fits the safety/autonomy framing.)

The `titleKey` and `descriptionKey` will fall back to the literal key strings if no translation is registered yet — that's fine for now (other entries use the same pattern; i18n can be added in a follow-up commit if needed). To avoid raw keys in the UI, use literal strings instead:

```tsx
  {
    id: 'autonomy',
    titleKey: undefined,
    descriptionKey: undefined,
    title: 'Agent autonomy',
    description: 'Tool action rate limits and safety thresholds.',
    route: 'autonomy',
    // ... icon as above ...
  },
```

BUT the existing render block (line 498-508) calls `t(item.titleKey)` directly — so the cleanest path is to register the i18n keys. Open `app/src/lib/i18n/locales/en.json` (or whichever file holds the existing `settings.developerMenu.*` keys — find it via `grep -rn 'settings.developerMenu.mcpServer' app/src/lib/i18n`) and add:

```json
"settings.developerMenu.autonomy.title": "Agent autonomy",
"settings.developerMenu.autonomy.desc": "Tool action rate limits and safety thresholds."
```

If the i18n file uses nested JSON, mirror the existing structure (drill into `settings → developerMenu → mcpServer` and add `autonomy` as a sibling object with `title` + `desc` keys).

- [ ] **Step 4: Typecheck + lint**

```bash
pnpm typecheck 2>&1 | tail -10
pnpm lint 2>&1 | tail -10
```

Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add app/src/components/settings/hooks/useSettingsNavigation.ts app/src/pages/Settings.tsx app/src/components/settings/panels/DeveloperOptionsPanel.tsx app/src/lib/i18n
git commit -m "feat(app): route + menu link for AutonomyPanel"
```

---

## Task 11: UI tests for `AutonomyPanel`

**Files:**
- Test: `app/src/components/settings/panels/__tests__/AutonomyPanel.test.tsx`

- [ ] **Step 1: Inspect a reference test for setup conventions**

Run:

```bash
ls app/src/components/settings/panels/__tests__/
```

Open one of the simpler existing panel tests (e.g. `MessagingPanel.test.tsx` or `NotificationsPanel.test.tsx`) to copy the mocking pattern for `tauriCommands/config`. Look for the `vi.mock('../../../../utils/tauriCommands/...')` setup at the top.

- [ ] **Step 2: Write the failing tests**

Create `app/src/components/settings/panels/__tests__/AutonomyPanel.test.tsx`:

```tsx
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, test, vi } from 'vitest';

vi.mock('../../../../utils/tauriCommands/config', () => ({
  openhumanGetAutonomySettings: vi.fn(),
  openhumanUpdateAutonomySettings: vi.fn(),
}));

import AutonomyPanel from '../AutonomyPanel';
import {
  openhumanGetAutonomySettings,
  openhumanUpdateAutonomySettings,
} from '../../../../utils/tauriCommands/config';

const mockGet = vi.mocked(openhumanGetAutonomySettings);
const mockUpdate = vi.mocked(openhumanUpdateAutonomySettings);

const renderPanel = () =>
  render(
    <MemoryRouter initialEntries={['/settings/autonomy']}>
      <AutonomyPanel />
    </MemoryRouter>
  );

describe('AutonomyPanel', () => {
  beforeEach(() => {
    mockGet.mockReset();
    mockUpdate.mockReset();
  });

  test('loads the current value on mount', async () => {
    mockGet.mockResolvedValue({ result: { max_actions_per_hour: 250 }, logs: [] });
    renderPanel();
    const input = (await screen.findByLabelText(/Max actions per hour/i)) as HTMLInputElement;
    await waitFor(() => expect(input).toHaveValue(250));
  });

  test('Save is disabled until the value changes', async () => {
    mockGet.mockResolvedValue({ result: { max_actions_per_hour: 20 }, logs: [] });
    renderPanel();
    const saveBtn = await screen.findByRole('button', { name: /^Save$/ });
    expect(saveBtn).toBeDisabled();

    const input = await screen.findByDisplayValue('20');
    fireEvent.change(input, { target: { value: '100' } });
    expect(saveBtn).not.toBeDisabled();
  });

  test('Save invokes the wrapper and shows confirmation', async () => {
    mockGet.mockResolvedValue({ result: { max_actions_per_hour: 20 }, logs: [] });
    mockUpdate.mockResolvedValue({
      result: { config: {}, workspace_dir: '/tmp', config_path: '/tmp/cfg.toml' },
      logs: [],
    });
    renderPanel();
    const input = await screen.findByDisplayValue('20');
    fireEvent.change(input, { target: { value: '300' } });
    fireEvent.click(screen.getByRole('button', { name: /^Save$/ }));
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith({ max_actions_per_hour: 300 })
    );
    await screen.findByText(/Saved\./i);
  });

  test('shows inline validation when the value is out of range', async () => {
    mockGet.mockResolvedValue({ result: { max_actions_per_hour: 20 }, logs: [] });
    renderPanel();
    const input = await screen.findByDisplayValue('20');
    fireEvent.change(input, { target: { value: '0' } });
    await screen.findByText(/Must be an integer between 1 and 10,000/i);
    expect(screen.getByRole('button', { name: /^Save$/ })).toBeDisabled();
  });

  test('surfaces RPC errors and reverts to the last committed value', async () => {
    mockGet.mockResolvedValue({ result: { max_actions_per_hour: 50 }, logs: [] });
    mockUpdate.mockRejectedValue(new Error('disk full'));
    renderPanel();
    const input = await screen.findByDisplayValue('50');
    fireEvent.change(input, { target: { value: '500' } });
    fireEvent.click(screen.getByRole('button', { name: /^Save$/ }));
    await screen.findByText(/Failed: disk full/);
    // Reverted to last committed value.
    expect(input).toHaveValue(50);
  });
});
```

- [ ] **Step 3: Run to verify they pass**

```bash
pnpm debug unit app/src/components/settings/panels/__tests__/AutonomyPanel.test.tsx
```

Expected: 5 tests pass. If `findByLabelText` fails, the test falls back to `findByDisplayValue`. If a different test helper is conventional in this codebase, check a neighbouring panel's test for the right imports — `app/src/test/setup.ts` may register `@testing-library/jest-dom`.

- [ ] **Step 4: Commit**

```bash
git add app/src/components/settings/panels/__tests__/AutonomyPanel.test.tsx
git commit -m "test(app): cover AutonomyPanel load/save/validate/error paths"
```

---

## Task 12: E2E case — persist through real core RPC

**Files:**
- Modify: `app/test/e2e/specs/settings-advanced-config.spec.ts`

- [ ] **Step 1: Add the E2E case**

Append inside the existing `describe('Settings - Advanced Config', …)` block, after the `'persists composio trigger triage settings'` test (around line 99):

```ts
  it('persists autonomy max_actions_per_hour through core RPC', async function () {
    this.timeout(60_000);
    const before = await callOpenhumanRpc('openhuman.config_get_autonomy_settings', {});
    expect(before.ok).toBe(true);

    await navigateViaHash('/settings/autonomy');
    await waitForText('Agent autonomy', 15_000);

    const input = await browser.$('#autonomy-max-actions');
    await input.waitForExist({ timeout: 10_000 });
    await input.setValue('250');
    await clickText('Save', 10_000);
    await waitForText('Saved', 10_000);

    await browser.waitUntil(
      async () => {
        const after = await callOpenhumanRpc('openhuman.config_get_autonomy_settings', {});
        return after.ok && after.result?.result?.max_actions_per_hour === 250;
      },
      { timeout: 15_000, interval: 500, timeoutMsg: 'autonomy setting did not persist' }
    );
  });
```

- [ ] **Step 2: Build the bundle, then run just this spec**

```bash
pnpm test:e2e:build
bash app/scripts/e2e-run-spec.sh test/e2e/specs/settings-advanced-config.spec.ts settings-advanced-config
```

Expected: all cases in this spec pass, including the new one. If the new case fails because the input id mismatches, check the `id="autonomy-max-actions"` attribute on the `<input>` in Task 9.

- [ ] **Step 3: Commit**

```bash
git add app/test/e2e/specs/settings-advanced-config.spec.ts
git commit -m "test(e2e): persist autonomy max_actions_per_hour through core RPC"
```

---

## Task 13: Final integration — coverage, full test sweep, manual smoke

- [ ] **Step 1: Run the changed-file unit test suites + Rust tests**

```bash
pnpm debug unit app/src/utils/tauriCommands/config.test.ts
pnpm debug unit app/src/components/settings/panels/__tests__/AutonomyPanel.test.tsx
pnpm debug rust autonomy
pnpm debug rust json_rpc_config_autonomy_settings_roundtrip
```

Expected: all pass.

- [ ] **Step 2: Lint + format**

```bash
pnpm lint 2>&1 | tail
pnpm format:check 2>&1 | tail
cargo fmt --manifest-path Cargo.toml -- --check 2>&1 | tail
```

If `format:check` complains, run `pnpm format` and amend the fixup into a single trailing commit.

- [ ] **Step 3: Coverage on changed lines**

The PR coverage gate is `≥80% on changed lines`. Quickly sanity-check:

```bash
pnpm test:coverage 2>&1 | tail -20
```

If a changed line in `AutonomyPanel.tsx` or `config.ts` isn't covered, add a focused test rather than padding existing ones.

- [ ] **Step 4: Manual smoke (HUMAN-IN-THE-LOOP)**

Don't skip this — the spec calls for it.

```bash
pnpm dev:app
```

In the running app:
1. Open `Settings → Developer Options → Agent autonomy`.
2. Confirm the current value loads (default 20 on a fresh workspace).
3. Change to 300, click Save → confirm "Saved" appears.
4. Reopen the panel → confirm 300 is shown.
5. Try entering 0 → confirm validation message, Save disabled.
6. Try entering 99999 → confirm validation message client-side.

Then verify the new value actually changes agent behavior:
1. Open a fresh chat with the agent and trigger more than 20 tool calls (e.g. a multi-step task).
2. With the limit at 300, the agent should not hit the "Rate limit exceeded" error.

Document the smoke result in the PR description.

- [ ] **Step 5: Push branch and open PR**

```bash
git push -u origin feat/ui-max-actions-per-hour
gh pr create --repo tinyhumansai/openhuman --head EvanCarson:feat/ui-max-actions-per-hour --base main \
  --title "feat(app): UI control for max_actions_per_hour (#2486)" \
  --body "$(cat <<'EOF'
## Summary
- Adds `config_get_autonomy_settings` / `config_update_autonomy_settings` JSON-RPC methods (mirrors the existing `config_*_meet_settings` pair).
- Surfaces them through a new `AutonomyPanel` linked from `Settings → Developer Options`. Number input with presets (20/100/500/1000); validates 1–10000.
- Persists to the user's `config.toml`. Takes effect on the next agent session — running sessions keep their current limit (documented in helper text).

Scoped from #2486 to the single `max_actions_per_hour` knob; the new panel is shaped so follow-up PRs can add `allowed_commands`, `auto_approve`, etc.

Pre-existing `openhuman.security_policy_info` bug (returns `SecurityPolicy::default()` instead of loaded config) is **not** fixed here — UI sidesteps it by reading from the new dedicated RPC. Separate follow-up.

## Test plan
- [ ] `pnpm debug rust autonomy` — Rust unit + roundtrip
- [ ] `pnpm debug rust json_rpc_config_autonomy_settings_roundtrip` — JSON-RPC E2E
- [ ] `pnpm debug unit app/src/utils/tauriCommands/config.test.ts` — TS wrapper unit tests
- [ ] `pnpm debug unit app/src/components/settings/panels/__tests__/AutonomyPanel.test.tsx` — UI unit tests
- [ ] `bash app/scripts/e2e-run-spec.sh test/e2e/specs/settings-advanced-config.spec.ts settings-advanced-config` — WDIO E2E
- [ ] Manual smoke in `pnpm dev:app` — load value, save, restart-free verify via new chat

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

(Replace `EvanCarson:` if the user's fork remote is named differently — check `git remote -v`.)

---

## Self-Review Notes

- **Spec coverage**: every "In scope" bullet has at least one task. Out-of-scope items (other autonomy fields, hot-reload, usage display, `security_policy_info` bug) are explicitly excluded in PR body + helper text.
- **Type consistency**: `max_actions_per_hour` is the field name everywhere — Rust struct field, JSON property, RPC param, TS wrapper arg, input id. RPC method names use the `config_*_autonomy_settings` shape consistently.
- **Schema gap noted**: the spec called for `TypeSchema::U32` implicitly but the type system has no `U32` variant — Task 3 documents the `U64` fallback (informational schema only; serde narrows on the DTO side).
- **Spec deviation called out**: the spec described the UI as "a subsection inside DeveloperOptionsPanel"; the plan creates a dedicated subpanel + route instead, matching every other entry in DeveloperOptionsPanel. Same UX intent; better extensibility for the follow-up autonomy fields.
