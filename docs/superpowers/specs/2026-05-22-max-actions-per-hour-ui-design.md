# Design: UI-configurable `max_actions_per_hour`

**Issue:** [tinyhumansai/openhuman#2486](https://github.com/tinyhumansai/openhuman/issues/2486) (scoped to one item).
**Date:** 2026-05-22.
**Status:** Draft (pre-implementation).

## Problem

The agent's tool action ceiling defaults to `max_actions_per_hour = 20`. Once exhausted, all subsequent tool calls are silently denied (`"Rate limit exceeded: action budget exhausted"`). For non-trivial sessions this is too low. Today, raising it requires hand-editing `~/.openhuman/.../config.toml` and restarting the core. There is no UI control.

The backend already supports the field — `AutonomyConfig.max_actions_per_hour` is loaded from TOML, defaults to 20, and threaded through `SecurityPolicy::from_config()` at every site that builds a policy (session builder, cron scheduler, channels runtime, MCP server, node runtime, local CLI). What's missing is a way to change it from the app.

## Scope

**In scope**
- A user-editable `max_actions_per_hour` field, persisted to the user's `config.toml`.
- A pair of JSON-RPC methods to read and write the value.
- A new "Agent autonomy" subsection inside the existing `DeveloperOptionsPanel`.
- Validation: `1 <= value <= 10_000`.

**Out of scope (deliberate)**
- Other autonomy fields raised in issue #2486 — `allowed_commands`, `auto_approve`, `block_high_risk_commands`, `max_cost_per_day_cents`. The new RPC and panel are shaped so these can be added later by extending the same patch struct + panel section.
- Hot-reload of running sessions / cron jobs / channels. New value applies to the *next* session; running policies keep their existing limit.
- Aggregated per-user usage display (`"X / Y used this hour"`). The action counter lives inside per-session `SecurityPolicy` instances, so there is no single number to display without first building one.
- Fixing the pre-existing `openhuman.security_policy_info` bug (returns `SecurityPolicy::default()` instead of the loaded config). Filed as a separate follow-up; this PR sidesteps it by not reading from that endpoint.

## Architecture

Four thin layers following the established `config_*_settings` pattern in this repo (e.g. `config_get_meet_settings` / `config_update_meet_settings`):

```
UI (React)
  DeveloperOptionsPanel — new "Agent autonomy" subsection
    │
    ▼  coreRpcClient → invoke('core_rpc_relay', …)
JSON-RPC controllers (src/openhuman/config/schemas.rs)
  handle_get_autonomy_settings    → openhuman.config_get_autonomy_settings
  handle_update_autonomy_settings → openhuman.config_update_autonomy_settings
    │
    ▼
Domain ops (src/openhuman/config/ops.rs)
  apply_autonomy_settings(&mut Config, AutonomySettingsPatch)
  load_and_apply_autonomy_settings(AutonomySettingsPatch)
    │
    ▼  config.save() → user TOML
Existing readers (unchanged)
  SecurityPolicy::from_config() in:
    - agent/harness/session/builder.rs
    - cron/scheduler.rs, cron/ops.rs
    - channels/runtime/startup.rs
    - mcp_server/tools.rs
    - runtime_node/ops.rs
    - tools/local_cli.rs
```

Each new construction of `SecurityPolicy` reads the current `Config`, so a saved change takes effect on the next session / cron tick / channel pickup without any propagation work.

## Components

### Rust core

**`src/openhuman/config/ops.rs`** — add (mirror `MeetSettingsPatch` / `apply_meet_settings`):

```rust
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct AutonomySettingsPatch {
    pub max_actions_per_hour: Option<u32>,
}

pub async fn apply_autonomy_settings(
    config: &mut Config,
    update: AutonomySettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    if let Some(v) = update.max_actions_per_hour {
        if v == 0 || v > 10_000 {
            return Err("max_actions_per_hour must be between 1 and 10000".into());
        }
        config.autonomy.max_actions_per_hour = v;
    }
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!("autonomy settings saved to {}", config.config_path.display())],
    ))
}

pub async fn load_and_apply_autonomy_settings(
    update: AutonomySettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_autonomy_settings(&mut config, update).await
}
```

**`src/openhuman/config/schemas.rs`** — add:

- `ControllerSchema` entries for `get_autonomy_settings` and `update_autonomy_settings`, registered in the controller list near the existing meet entries (~line 286).
- Schema definitions in the `schemas(name)` match block (~line 672) — `get_autonomy_settings` takes no params; `update_autonomy_settings` takes `{ max_actions_per_hour?: u32 }`.
- `handle_get_autonomy_settings` returns `{ "max_actions_per_hour": <current value> }` from a loaded `Config`.
- `handle_update_autonomy_settings` deserialises into an `AutonomySettingsUpdate` DTO, builds the patch, calls `load_and_apply_autonomy_settings`.

Both handlers follow the existing `debug!("[config][rpc] X enter") / ok / failed` logging pattern.

Resulting RPC method names:
- `openhuman.config_get_autonomy_settings`
- `openhuman.config_update_autonomy_settings`

### Tauri / TypeScript

**`app/src/utils/tauriCommands/config.ts`** — add `getAutonomySettings()` and `updateAutonomySettings(patch)` wrappers, mirroring the meet-settings wrappers.

**`app/src/services/rpcMethods.ts`** — add the two new method constants.

**`app/src/components/settings/panels/DeveloperOptionsPanel.tsx`** — append an "Agent autonomy" subsection:
- Heading + helper text: *"Maximum tool actions an agent can run per hour. New value applies to your next chat — running sessions keep their current limit."*
- Number `<input>` with `min=1`, `max=10000`, integer step.
- Preset chips: `20 (default)`, `100`, `500`, `1000`.
- Save button — disabled when value is unchanged or invalid.
- Inline confirmation on save; inline error message on failure.

The panel fetches the current value via `getAutonomySettings()` on mount; on save, calls `updateAutonomySettings({ max_actions_per_hour })`. On success, keeps the edited value as the new committed state and shows confirmation; on error, reverts the UI to the last committed value and shows the error message.

## Data flow

**Save**
1. User edits → clicks Save in `DeveloperOptionsPanel`.
2. `updateAutonomySettings({ max_actions_per_hour: 200 })` → `core_rpc_relay` → core.
3. `handle_update_autonomy_settings` → `load_and_apply_autonomy_settings` → mutates `config.autonomy.max_actions_per_hour` → `config.save()` writes user TOML.
4. RPC returns `RpcOutcome { value: snapshot_json, logs: ["autonomy settings saved to <path>"] }`.
5. UI shows inline "Saved" confirmation.

**Read**
1. Panel mounts → `getAutonomySettings()` → `openhuman.config_get_autonomy_settings`.
2. `handle_get_autonomy_settings` calls `load_config_with_timeout()` → returns `{ max_actions_per_hour: config.autonomy.max_actions_per_hour }`.
3. UI initialises field state with returned value.

## Error handling

- **Invalid input** (≤0, >10000, non-integer): rejected client-side first via `min`/`max` attributes; re-validated in the handler — returns `Err("max_actions_per_hour must be between 1 and 10000")`. UI surfaces the message inline.
- **Config load timeout / disk write failure**: propagates as RPC error; panel shows the message inline; existing TOML on disk is unchanged.
- **Core not yet ready**: panel handles this the same way other panels do — loading skeleton, retry on RPC error.
- **Edits do not affect running sessions**: documented in the panel's helper text. This is expected behavior, not a failure mode — no warning surfaced.

## Logging

Per repo rule (verbose diagnostics on new/changed flows, stable grep-friendly prefixes):

- `[config][rpc] update_autonomy_settings enter max_actions_per_hour=<n>`
- `[config][rpc] update_autonomy_settings ok` / `... failed: <err>`
- `[config][rpc] get_autonomy_settings enter`
- `[config][rpc] get_autonomy_settings ok max_actions_per_hour=<n>` / `... failed: <err>`

## Testing

**Rust unit (`src/openhuman/config/ops_tests.rs`)** — alongside the `apply_meet_settings` tests:
- `apply_autonomy_settings_persists_max_actions_per_hour` — assert config mutated + saved to disk.
- `apply_autonomy_settings_no_op_when_patch_empty` — `None` patch leaves the value unchanged.
- `apply_autonomy_settings_rejects_zero` and `_rejects_above_cap` — validation works at both bounds.
- `load_and_apply_autonomy_settings_roundtrip` — load → apply → reload → value matches.

**Rust handler (`src/openhuman/config/schemas_tests.rs`)**:
- `update_autonomy_settings` and `get_autonomy_settings` route through the controller registry and return the expected JSON shape.
- Invalid params return `Err`.

**Rust E2E (`tests/json_rpc_e2e.rs`)**:
- New test: post `openhuman.config_update_autonomy_settings` then `openhuman.config_get_autonomy_settings`; assert the round-trip over actual JSON-RPC.

**TypeScript unit (`app/src/utils/tauriCommands/config.test.ts`)**:
- `getAutonomySettings` invokes the correct method.
- `updateAutonomySettings` passes the patch correctly.

**UI (`app/src/components/settings/panels/__tests__/DeveloperOptionsPanel.test.tsx`)**:
- Loads current value on mount.
- Save button disabled until value changes and is valid.
- Save calls the wrapper with the new value, shows confirmation.
- Validation error surfaces inline.

**E2E (`app/test/e2e/specs/settings-advanced-config.spec.ts`)** — add a case to the existing spec rather than a new file:
- Open Developer Options, change the rate-limit field, save, reopen, confirm persisted value.

Coverage on changed lines must meet the repo's ≥80% merge gate.

## File touch list

```
src/openhuman/config/ops.rs              (+ ~30 lines, mirror meet pattern)
src/openhuman/config/ops_tests.rs        (+ ~80 lines, new tests)
src/openhuman/config/schemas.rs          (+ ~60 lines: schema, registration, handlers)
src/openhuman/config/schemas_tests.rs    (+ ~40 lines)
tests/json_rpc_e2e.rs                    (+ ~30 lines, round-trip test)

app/src/utils/tauriCommands/config.ts    (+ ~25 lines, two wrappers)
app/src/utils/tauriCommands/config.test.ts (+ ~30 lines)
app/src/services/rpcMethods.ts           (+ 2 lines, method constants)
app/src/components/settings/panels/DeveloperOptionsPanel.tsx (+ ~80 lines, new section)
app/src/components/settings/panels/__tests__/DeveloperOptionsPanel.test.tsx (+ ~60 lines)
app/test/e2e/specs/settings-advanced-config.spec.ts (+ ~30 lines, new case)
```

No new files; no schema-breaking changes to existing handlers; no changes to `SecurityPolicy`, `from_config`, or any consumer of those.

## Risks & open questions

- **Stale running sessions** — a user who hits the ceiling, raises the limit, and expects the *current* chat to recover will be confused. Mitigated by helper text. If this turns out to be a common complaint, Approach C (live propagation via event bus) is the follow-up.
- **`security_policy_info` returns defaults** — pre-existing bug, deferred. The UI does not read from it.
- **Cap of 10,000** — chosen as "effectively unlimited for human use" while bounding the field against typos. Easy to lift if needed.

## Sequencing

1. Rust ops + schemas + their unit tests.
2. Rust E2E round-trip test.
3. TS wrappers + their unit tests.
4. UI panel section + UI tests.
5. E2E spec case.
6. Manual smoke (start dev app, change value, restart-free verify by starting a new agent session).
