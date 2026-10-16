# config

The configuration layer for the core. It defines the `Config` struct that maps to a user's `config.toml`,
decides which `config.toml` to read for the current user and workspace, overlays environment variables
and CLI launch flags on top of it, runs startup data migrations, and writes it back atomically. It also
exposes the `config.*` RPC namespace the settings UI uses to read and change those settings.

Nearly every other domain reads its slice of settings from here (`config.agent`, `config.memory`,
`config.channels`, `config.autonomy`, and so on), usually through `config::load_config_with_timeout()`.

## How it works

### Where the config lives

Every user gets a directory under a shared root. The root is `~/.openhuman`, or `~/.openhuman-staging`
when the app environment is `staging` ([`app_env.rs`](./app_env.rs) reads `OPENHUMAN_APP_ENV` / `VITE_OPENHUMAN_APP_ENV`,
first at runtime, then as baked in at compile time).

```text
~/.openhuman/
  active_user.toml          which user is signed in (shared by all users)
  active_workspace.toml     optional marker pointing at a config dir
  users/
    local/                  pre-login profile (PRE_LOGIN_USER_ID)
      config.toml
      workspace/
    <user-id>/
      config.toml
      config.toml.bak       previous version, kept by every save
      workspace/            agent state: memory, sessions, skills, ...
```

[`schema/load/dirs.rs`](./schema/load/dirs.rs) resolves the pair `(config dir, workspace dir)` in a fixed order. The first match
wins, and the winner is recorded as a `ConfigResolutionSource` in the load logs.

```text
OPENHUMAN_WORKSPACE set?  --yes--> resolve_config_dir_for_workspace(path)
        | no
        v
active_user.toml names a user?  --yes--> users/<id>/{config.toml, workspace}
        | no
        v
active_workspace.toml present?  --yes--> <config_dir>/{config.toml, workspace}
        | no
        v
users/local/{config.toml, workspace}     (pre-login, DefaultConfigDir)
```

`resolve_config_dir_for_workspace` accepts both layouts an `OPENHUMAN_WORKSPACE` value can point at: a
directory that holds its own `config.toml`, or a `.../workspace` directory whose parent holds it. When the
resolved config dir has no `config.toml`, the loader warns, because every setting silently falls back to
schema defaults in that case.

The active-user read is strict on purpose (`read_active_user_id_checked_async`). A transient read failure
on an existing `active_user.toml` fails the load instead of falling through to `users/local`, which would
boot a signed-in user into an empty profile.

The agent's action directory (`Config::action_dir`, where acting tools read and write) is separate from
the workspace. It comes from `action_dir_override` in the file, then `OPENHUMAN_ACTION_DIR`, then the
default projects directory (`OPENHUMAN_PROJECTS_DIR`, else `~/OpenHuman/projects`). Agent deliverables
go to `~/OpenHuman/projects/Files` unless overridden (`resolve_files_dir`).

### Loading

`Config::load_or_init()` in [`schema/load/impl_load.rs`](./schema/load/impl_load.rs) is the authoritative startup path. After resolving
the directories it takes one of three branches:

```text
resolve dirs
    |
    +-- pre-login and no config.toml --> in-memory defaults, nothing written
    |
    +-- config.toml exists ----------> load_existing_config  (branches.rs)
    |      fix world-readable mode to 0600 (only if we own the file)
    |      read with retry; non-UTF-8 -> rename to .corrupted.<ts>, try .bak
    |      parse (recover to defaults on bad TOML)
    |      inline fix-ups: legacy inference URL, memory backend,
    |        cloud provider slugs, search settings, memory sources
    |      apply env + CLI overrides
    |      if corrupted: persist the recovered config
    |      migrations::run_pending
    |      decrypt secrets (upgrade legacy enc: to enc2:, then save)
    |
    +-- no config.toml yet ----------> init_new_config
           defaults stamped at CURRENT_SCHEMA_VERSION
           migrations::seed_new_workspace (managed cloud provider)
           save, chmod 0600, apply overrides, run_pending
```

On success the resolved workspace is published to the cache in [`schema/load/active_workspace.rs`](./schema/load/active_workspace.rs), so
`active_workspace_dir_cached()` can answer from synchronous code (the Event Log stamps every event this
way). `active_workspace_snapshot()` resolves the workspace again from disk and returns it with a revision
number, for callers that make one decision per workspace switch.

Two other entry points exist for narrower cases. `Config::load_from_config_path` reloads a config whose
paths are already known and ignores `OPENHUMAN_WORKSPACE`, for long-lived objects that hold a snapshot.
`Config::load_from_default_paths` ignores `OPENHUMAN_WORKSPACE` too and is read-only; the debug prompt
dump uses it.

Most code does not call these directly. `ops::load_config_with_timeout()` wraps `load_or_init` in a
30-second timeout, returns the embedding host's `Config` instead when one is installed on the current
`CoreContext`, and seeds per-token prices into the model registry in memory. Code handed a specific
workspace should call `load_config_for_workspace_with_timeout(workspace_dir)` so it does not pick up the
process-global user's settings.

### Overrides

Overrides are applied after parsing and never written back to disk.

1. `Config::apply_env_overrides` ([`schema/load/env_overlay.rs`](./schema/load/env_overlay.rs) plus one submodule per section) reads
   `OPENHUMAN_*` variables such as `OPENHUMAN_MODEL` and `OPENHUMAN_TOOL_DISPATCHER`. Only namespaced
   names are honored. Tests call the pure `apply_env_overlay_with` with a map-backed `EnvLookup`
   ([`schema/load/env.rs`](./schema/load/env.rs)) so they do not touch the process environment.
2. The same call publishes process-wide state: the proxy settings go to `set_runtime_proxy_config`
   (and into the process environment when `proxy.scope` is `Environment`), and the embedding rate limit
   goes to `tinyinference_embeddings`.
3. CLI launch flags (`openhuman -p <provider> -m <model>`) are applied last by [`schema/cli_overrides/`](./schema/cli_overrides/).
   [`core/cli.rs`](../core/cli.rs) registers them with `set_cli_inference_overrides`, and `save` restores the persisted
   values before serializing so a launch flag never lands in `config.toml`.

### Saving

`Config::save()` clones the config, restores CLI-overridden fields, encrypts secrets with the keyring's
`SecretStore` ([`schema/load/secrets.rs`](./schema/load/secrets.rs), `enc2:` ChaCha20-Poly1305 values), serializes to TOML, and
writes a temp file that is chmod 0600 before any secret byte is written. It then fsyncs and hands off to
[`schema/load/atomic_commit.rs`](./schema/load/atomic_commit.rs), which copies the old file to `config.toml.bak` and renames the temp file
into place. The live config is untouched unless the commit succeeds. That is the file source. On a shared (multi-tenant) storage
backend `save` writes the scope's `config/{scope}` document instead: the secrets are not field-encrypted (the whole body is sealed with
the scope's data key), there is no temp file or `.bak`, and the `[storage]` table is never stored. See
[`schema/load/source/`](./schema/load/source/mod.rs).

### Migrations

`migrations::run_pending` compares `Config::schema_version` against `CURRENT_SCHEMA_VERSION` (13) and
runs each one-shot migration whose gate the workspace has not crossed yet. Failures are logged and retried
on the next launch; they never block startup. A fresh workspace is born at the current version, which is
why the managed provider is seeded at creation by `seed_new_workspace` rather than by a migration. See
[migrations/README.md](migrations/README.md).

### Changing settings over RPC

Each settings group follows the same pattern. A handler in [`schemas/controllers/`](./schemas/controllers/) deserializes params into
an update struct from [`schemas/helpers.rs`](./schemas/helpers.rs), converts it into a `*Patch` type, and calls
`ops::load_and_apply_*`. That function loads the config, applies the patch (`apply_*`), saves, and returns
an `Outcome<serde_json::Value>`. Business rules for each group (validation, model id checks, path
normalization) live in `ops/`, not in the handlers.

```text
config.update_model_settings (JSON-RPC / CLI)
  -> schemas/controllers/inference.rs  handle_update_model_settings
  -> ops::load_and_apply_model_settings(ModelSettingsPatch)
  -> load_config_with_timeout -> apply_model_settings -> Config::save
  -> Outcome<Value>
```

## Layout

| Path | What it does |
| --- | --- |
| [`mod.rs`](./mod.rs) | Module declarations and the flat public API: re-exports the section types, path helpers, proxy helpers and model constants from `schema`, all of `ops` (also as `config::rpc`), and the controller lists. |
| `schema/` | The `Config` struct ([`schema/types/config.rs`](./schema/types/config.rs)), one file per section, defaults, the proxy client, CLI overrides, and the whole load/save pipeline under [`schema/load/`](./schema/load/). See [schema/README.md](schema/README.md). |
| `ops/` | Settings operations behind the RPC surface: config snapshots, runtime flags, data paths, `reset_local_data`, and the `apply_*` / `load_and_apply_*` pairs per settings group. See [ops/README.md](ops/README.md). |
| `schemas/` | Controller schemas and thin handlers for the `config` namespace. `schema_defs/` holds the `ControllerSchema` definitions by group, `controllers/` the handlers, and `controllers/registry.rs` the lists. |
| `migrations/` | Version-gated startup migrations and `seed_new_workspace`. See [migrations/README.md](migrations/README.md). |
| `workspace/` | Workspace bootstrap (`init_workspace`) and the `workspace.*` RPCs over the editable persona files (`SOUL.md`, `IDENTITY.md`). Registers its own controllers. See [workspace/README.md](workspace/README.md). |
| `app_env.rs` | Reads the app environment (`production`, `staging`) used to pick the root directory. |
| [`daemon.rs`](./daemon.rs) | `DaemonConfig`, a small bundle of `data_dir`, `workspace_dir` and the autonomy, security, reliability, secrets and audit sections, built from a Tauri app data dir by `from_app_data_dir`. Re-exported by `openhuman-embed`. |
| [`tools.rs`](./tools.rs) | Read-only agent tools over config: `config_snapshot`, `config_get_runtime_flags`, `config_get_client_config`, `config_get_autonomy`, `config_get_search`, `config_resolve_api_url`, `config_get_data_paths`. |
| [`workspace_handle.rs`](./workspace_handle.rs) | `workspace_handle(path)`, a short stable digest of a workspace path. Process-wide streams (Event Log, `core_notification`) carry it instead of the path so home directories never reach exported logs. |

## Key types and entry points

| Symbol | Where | What it is for |
| --- | --- | --- |
| `Config` | `schema/types/config.rs` | The whole persisted document, plus runtime-only fields such as `config_path`, `workspace_dir`, `action_dir` and `recovered_from_corruption`. |
| `Config::load_or_init` | `schema/load/impl_load.rs` | Resolve, read, recover, migrate and decrypt the active user's config. |
| `Config::save` | `schema/load/impl_load.rs` | Encrypt secrets and replace `config.toml` atomically, keeping a `.bak`. |
| `load_config_with_timeout` | [`ops/loader/load.rs`](./ops/loader/load.rs) | What domain code should call: timeout, embedder override, registry pricing. |
| `load_config_for_workspace_with_timeout` | `ops/loader/load.rs` | Load the config that belongs to a given workspace. |
| `active_workspace_snapshot`, `active_workspace_dir_cached` | `schema/load/dirs.rs`, `schema/load/active_workspace.rs` | Which workspace is active now, async with revision or cached and synchronous. |
| `user_openhuman_dir`, `pre_login_user_dir`, `write_active_user_id`, `clear_active_user` | [`schema/load_user_state.rs`](./schema/load_user_state.rs) | Per-user directory layout and the `active_user.toml` marker. |
| `build_runtime_proxy_client`, `apply_runtime_proxy_to_builder` | [`schema/proxy.rs`](./schema/proxy.rs) | `reqwest` clients that honor the configured proxy for a named service key. |
| `migrations::run_pending`, `CURRENT_SCHEMA_VERSION` | [`migrations/mod.rs`](./migrations/mod.rs) | The startup migration runner. |
| `DEFAULT_MODEL`, `MODEL_MANAGED_DEFAULT`, `LEGACY_TIER_MODELS`, `WORKLOAD_ROLES` | [`schema/types/model_ids.rs`](./schema/types/model_ids.rs) | Model id constants shared by inference and the settings UI. |
| `DaemonConfig` | `daemon.rs` | Supervisor-level config bundle for embedders. |

## RPC surface

All methods are in the `config` namespace and are registered by [`core/all.rs`](../core/all.rs) through
`all_config_registered_controllers()` under `DomainGroup::Config`. There are 41.

| Group | Methods |
| --- | --- |
| Whole config | `get_config`, `get_client_config`, `get_runtime_flags`, `resolve_api_url`, `get_data_paths`, `reset_local_data`, `agent_server_status`, `get_dashboard_settings` |
| Inference and memory | `update_model_settings`, `update_memory_settings`, `update_local_ai_settings`, `update_runtime_settings` |
| Agent | `get_agent_settings`, `update_agent_settings`, `get_agent_paths`, `update_agent_paths`, `get_autonomy_settings`, `update_autonomy_settings`, `get_sandbox_settings`, `update_sandbox_settings`, `update_computer_settings` |
| Privacy and search | `get_privacy_mode`, `set_privacy_mode`, `get_search_settings`, `update_search_settings` |
| Browser | `update_browser_settings`, `set_browser_allow_all` |
| Integrations | `get_composio_trigger_settings`, `update_composio_trigger_settings` |
| Voice | `get_dictation_settings`, `update_dictation_settings`, `get_voice_server_settings`, `update_voice_server_settings` |
| Onboarding and UI | `get_onboarding_completed`, `set_onboarding_completed`, `workspace_onboarding_flag_exists`, `workspace_onboarding_flag_set`, `get_analytics_settings`, `update_analytics_settings`, `get_user_timezone`, `update_user_timezone` |

The `workspace` namespace (`file_read`, `file_write`, `file_reset`) is registered separately from
`workspace/`. The read-only agent tools in `tools.rs` and [`workspace/tools.rs`](./workspace/tools.rs) are re-exported through
[`crates/openhuman-core/src/tools/mod.rs`](../tools/mod.rs).

## Boundaries

- Secret encryption is done by `security::secret_storage::SecretStore`; this folder only decides which fields
  are secrets and when to encrypt or decrypt them.
- The autonomy policy is defined here as data (`AutonomyConfig`, `[autonomy] enabled = false` by
  default) but enforced by [`crates/openhuman-core/src/security/`](../security/) (`SecurityPolicy::from_config`).
- Hosted backend URLs and their environment overrides belong to the host crate that registers the config enricher
  ([`crates/openhuman-tinyhumans/src/backend/url.rs`](../../../openhuman-tinyhumans/src/backend/url.rs)). `config.api_url` is only the user's stored value.
- Credentials and sign-in are not config. The core receives a credential through `auth.set_credential`;
  login and `/auth/me` live in the host's session owner (`openhuman_tinyhumans::session`).
- Embedding hosts can supply their own `Config` (`CoreBuilder::config`), in which case
  `load_config_with_timeout` and `active_workspace_snapshot` return it instead of reading disk.
- Prices for the model registry come from `crates/openhuman-core/src/platform/cost/catalog`.

## Gotchas

- Do not cache a `Config` across a user or workspace switch. The active workspace is a marker on disk
  that can change while the process runs; use `active_workspace_snapshot` per decision.
- An `OPENHUMAN_WORKSPACE` that points inside `~/.openhuman` (rather than at a user's `workspace/` or a
  folder with its own `config.toml`) resolves to a config dir with no file, and every setting reverts to
  defaults. Look for the warning in the logs.
- Env and CLI overrides are runtime-only. A value that appears in `get_config` may not be in
  `config.toml`, and `save` will not persist it.
- A new workspace starts at `CURRENT_SCHEMA_VERSION`, so a migration never runs on it. Anything a fresh
  install must have belongs in `seed_new_workspace` or the defaults, not only in a migration.
- `load_or_init` and `save` box their inner futures to keep poll frames small in debug builds. Keep that
  pattern when adding branches.
- `Config` has no container-level `#[serde(default)]`. New fields need their own `#[serde(default)]`
  or older files fail to parse.

## Tests

Tests sit beside their modules as `*_tests.rs` files (`ops_*_tests.rs`, [`schemas_tests.rs`](./schemas_tests.rs),
`schema/load_*_tests.rs`, one per section under `schema/`, and so on). Tests that mutate
`OPENHUMAN_WORKSPACE` share `TEST_ENV_LOCK` from `mod.rs`.

```bash
cargo test -p openhuman config::
pnpm debug rust config::
```

## Further reading

- [Parent module README](../../README.md)
- [Settings](../../../../docs/gitbooks/en/features/settings.md)
- [Deep architecture reference](../../../../docs/gitbooks/en/developing/architecture.md)
