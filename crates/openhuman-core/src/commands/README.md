# commands

The `commands` domain answers one question for the chat composer: what can the
slash-command menu offer right now? It has a single read-only controller,
`commands.list`, that merges the core's fixed built-in slash commands with the
live skill and workflow catalogs. The frontend calls it once
(`openhuman.commands_list` from
[`app/src/features/conversations/aui/useSlashCommandSource.ts`](../../../../app/src/features/conversations/aui/useSlashCommandSource.ts)) instead of
making three separate calls.

Listing a command here never runs it. Each entry tells the frontend how to
dispatch it, and execution stays on the RPCs that already exist for that
kind of command.

## How it works

`ops::commands_list` builds the list in a fixed order: built-ins first, then
skills, then workflows.

```text
commands.list
    |
    v
ops::commands_list
    |
    +-- builtin_entries()            BUILTINS table, always present
    |
    +-- skills.list                  invoked in-process through
    |     (include_skills = true)    skills::all_skills_registered_controllers
    |
    +-- flows.list                   only with feature "flows", through
    |                                flows::all_flows_registered_controllers
    v
CommandsListResponse { commands: [CommandEntry, ...] }
```

The built-ins come from the `BUILTINS` table in [`ops.rs`](./ops.rs): `/new`, `/clear`,
`/plan`, `/build`, `/goal`, `/todo` and `/stop`. For each one the `id` is the
bare name (`"new"`), while `label` and `insert` keep the leading slash
(`"/new"`). The frontend inserts the `insert` text into the composer, and the
command then goes through whatever RPC the frontend already uses for it (for
example `/plan` maps to `agent.set_run_mode`).

Skills and workflows are fetched by looking up the other domain's registered
controller and calling its handler directly. This module reads only the JSON
those handlers return (the `skills` or `flows` array, and each item's `id`,
`name` and `description`), so it has no compile-time dependency on either
domain's Rust types. The skills call passes `include_skills: true` so the menu
lists capability skills under `skills/` roots as well as the `workflows/` root
automations. Skill and workflow entries carry `insert: None`; the frontend
dispatches them through `skills.run` or `flows.run`.

Each catalog fetch is best effort. If a controller is missing or its handler
returns an error, `invoke` logs at debug level (`[commands] ... lookup failed`)
and that catalog is left out. A broken skills or flows catalog never takes the
whole palette down, and the built-ins are always returned.

## Layout

| Path | What it does |
| --- | --- |
| [`mod.rs`](./mod.rs) | Module declarations, re-exports of `CommandEntry` and `CommandKind`, and `all_commands_registered_controllers` for the registry. |
| [`types.rs`](./types.rs) | Wire types: `CommandKind`, `CommandEntry`, `CommandsListResponse`. |
| `ops.rs` | The `BUILTINS` table, the in-process controller lookup, and `commands_list`. |
| [`schemas.rs`](./schemas.rs) | The `commands.list` controller schema and its thin handler. |

## Key types and entry points

- `CommandEntry` (`types.rs`) is one menu row: `id`, `label`, `description`,
  `kind`, and an optional `insert`. `description` is always present on the
  wire, as an empty string when the source has none, so the frontend never
  has to handle a missing key. `insert` is omitted when it is `None`.
- `CommandKind` (`types.rs`) is `builtin`, `skill` or `workflow` (snake_case
  on the wire).
- `CommandsListResponse` (`types.rs`) wraps the list as
  `{"commands": [...]}`, matching the `{"skills": [...]}` and
  `{"flows": [...]}` shapes of the catalogs it merges.
- `commands_list` (`ops.rs`) is the business operation and returns
  `Outcome<CommandsListResponse>`.
- `all_commands_registered_controllers` (`mod.rs`) is what
  [`core/all.rs`](../core/all.rs) pushes into the controller registry.

## RPC / CLI surface

| Method | Description |
| --- | --- |
| `commands.list` (wire `openhuman.commands_list`) | No inputs. Returns `commands`, an array of `{id, label, description, kind, insert?}`. |

The controller is registered under `DomainGroup::Agent` in `core/all.rs`. It
is part of the chat surface and always on.

## Boundaries

- Running a command is not handled here. Built-ins run through their own RPCs
  (`agent.set_run_mode` and friends), skills through `skills.run`, workflows
  through `flows.run`.
- The skill catalog belongs to `crate::skills`, and the workflow catalog to
  `crate::flows` (backed by the `tinyflows` submodule).
- The menu UI, its local command registry and the merge with
  frontend-only commands live in the app (`useSlashCommandSource.ts` and
  [`app/src/lib/commands/`](../../../../app/src/lib/commands/)).

## Gotchas

- The `flows.list` call is behind `#[cfg(feature = "flows")]`. Builds with
  `default-features = false` and no `flows` feature still compile, and the
  palette then lists only built-ins and skills.
- The handlers are called directly, not through JSON-RPC dispatch, so their
  result is unwrapped with `core::unwrap_rpc` before the array is read.
- An item without a string `id` is skipped. A missing or empty `name` falls
  back to the `id` as the label.

## Tests

Tests sit beside each module ([`ops_tests.rs`](./ops_tests.rs), [`schemas_tests.rs`](./schemas_tests.rs),
[`types_tests.rs`](./types_tests.rs)). Run them with `cargo test -p openhuman commands::` or
`pnpm debug rust commands::`.

## Further reading

- [Parent module README](../../README.md)
- [Deep architecture reference](../../../../docs/gitbooks/en/developing/architecture.md)
- [Chat](../../../../docs/gitbooks/en/features/chat.md)
