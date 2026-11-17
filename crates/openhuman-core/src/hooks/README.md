# Hooks

OpenHuman configures external hooks through the contracts owned by
`tinyagents-hooks`. The core keeps only the process-wide embedder registries in
[`agent/hooks.rs`](../agent/hooks.rs), implements `HookIdentity` for
`OpenHumanRunContext`, and installs the configured command `HookEngine` from
[`ops.rs`](./ops.rs).

Tool callbacks and their scoped registration live in `tinyagents-hooks` and
`tinyagents-runtime`. OpenHuman supplies its run context so a hook can inspect
the session, agent and working directory without introducing another hook
adapter. Post-turn learning uses the upstream `LearningSink` contract and
receives a privacy-preserving `TurnSummary` after the session commits.

The `hooks.*` RPC namespace and its model-facing command names remain stable.
Configuration parsing and command execution are supplied by
`tinyagents_hooks::command`; this module retains the OpenHuman controllers and
their lifecycle integration.
