# skills/catalog

Module path: `crate::skills::catalog`. RPC namespace: `skill_registry`, a
stable wire contract left unchanged by the module rename (JSON-RPC methods are
still `openhuman.skill_registry_<function>`, the CLI namespace is still
`skill_registry`; see `tests/in_process/skill_registry_e2e.rs`).

Hosts the `tinyskills` skill registry (`tinyskills::SkillRegistry`, the
`registry` feature) and adapts it to OpenHuman's RPC, agent tools and install
policy. The registry owns catalog fetching, the on-disk catalog store,
freshness, ranking, paging, facets, entry ids and `SKILL.md` resolution; this
module owns the HTTP transport, configuration, observability and installing
into the user skills directory.

- One process-wide registry (`registry::skill_registry`), configured from the
  environment and rebuilt when that configuration changes.
- A `reqwest` transport (`ReqwestTransport`) that honours the registry's
  contract: it connects only to the addresses the registry pinned, returns
  redirects to the registry's guard instead of following them, uses no proxy,
  and streams bodies within the registry's limits.
- The catalog is warmed asynchronously on core load. Reads serve the held
  catalog (`cached`) while a refresh runs and keep serving it, with
  `last_error`, when the registry cannot be reached. A failed refresh is not
  attempted again for 45 seconds (`REFRESH_COOLDOWN`, set through the
  builder's `RegistryTimeouts`), or longer when the upstream sends
  `Retry-After`.
- Entries without a `SKILL.md` download (LobeHub agents, portal pages) are
  `installable: false` and carry their `source_url`; installing one fails with
  `SKILL_REGISTRY_NO_DIRECT_DOWNLOAD` naming that page.
- Install catalog entries into the user skills directory; uninstall user-scope
  skills.
- Host the built-in `skill_setup` agent.

## Key files

| File | Purpose |
| --- | --- |
| [`mod.rs`](./mod.rs) | Feature gate (`skills` Cargo feature) and module wiring; re-exports the controller aggregators, `skill_registry`, `ReqwestTransport` and `HERMES_REGISTRY_ID` |
| [`registry.rs`](./registry.rs) | `RegistryConfig` (environment knobs below), the process registry handle |
| [`store.rs`](./store.rs) | `HostCatalogStore`, the registry's `CatalogStore` on the storage backend (`tinyskills_catalogs/<registry>` documents under the acting scope); imports a legacy `<cache dir>/<registry>.json` once, never modifies it |
| [`transport.rs`](./transport.rs) | `ReqwestTransport`, the `tinyskills::RegistryTransport` implementation (pinned addresses, no redirects, no proxy, cached clients) |
| [`ops.rs`](./ops.rs) | Boot warm-up, paged browse/search, facets, detail, `install_from_catalog`, registry error messages and Sentry reporting |
| [`tools.rs`](./tools.rs) | LLM-callable tools `skill_registry_browse`, `skill_registry_search`, `skill_registry_sources`, `skill_registry_install`, `skill_registry_uninstall` |
| [`types.rs`](./types.rs) | `RegistryCatalogEntry`, `CatalogQuery`, `CatalogPage`, `CatalogDetail`, and re-exported `tinyskills` registry types |
| [`schemas/controller_schemas.rs`](./schemas/controller_schemas.rs) | `skill_registry_*` `ControllerSchema` definitions and the registered-controller table |
| [`schemas/handlers.rs`](./schemas/handlers.rs) | Thin RPC handlers dispatching into `ops.rs` |
| [`schemas/wire_types.rs`](./schemas/wire_types.rs) | Request/response payload types for the handlers |
| [`stub.rs`](./stub.rs) | No-op facade compiled in when the `skills` feature is off |
| [`agent/skill_setup/`](./agent/skill_setup/) | Built-in `skill_setup` agent (`agent.toml`, `prompt.md`, `prompt.rs`) |

## RPC surface

Functions in `schemas/controller_schemas.rs::all_skill_registry_controller_schemas`,
all under the `skill_registry` namespace:

- `browse`: one page of catalog entries. Params: `page` (1-based),
  `page_size` (default 25, max 100), `source`/`sources`, `category`/`categories`,
  `force_refresh`. Returns `entries`, `total`, `page`, `page_size`,
  `total_pages`, `freshness` (`live` / `cached` / `local_fallback`),
  `fetched_at`, `refreshing` and `last_error`. Without `page` or `page_size` it
  returns every match in one page, built from summaries only (empty
  `download_url`; `installable` is authoritative).
- `search`: `browse` with a ranked `query`.
- `detail`: one entry by `entry_id`, with its `overview` and resolved
  `download_url`.
- `sources`: upstream sources present in the catalog, with per-source counts
  (`facets`) and the catalog's `freshness`.
- `categories`: categories present in the catalog, with counts and freshness.
- `install`: install a catalog entry by `entry_id` into user scope. The result
  carries `status`: `installed` with `url`, `stdout`, `stderr` and
  `new_skills`, or `scan_blocked` with `target`, `fetched_from`, `slug`,
  `digest`, `findings` and `message` when the supply-chain scan refused it.
  Passing that `digest` back as `acknowledged_digest` installs that document;
  only the Skills UI sends it, after the user chose "Install anyway".
- `uninstall`: remove an installed user-scope skill by slug.
- [`schemas`](./schemas): return the `skill_registry` controller schemas (CLI/RPC smoke-test generation).

Registry failures are reported as `SKILL_REGISTRY_<KIND>: <message>`, where
`<KIND>` is the upper-cased `tinyskills::RegistryErrorKind` (`TIMEOUT`,
`UNAVAILABLE`, `RATE_LIMITED`, `NOT_FOUND`, `NO_DIRECT_DOWNLOAD`,
`UPSTREAM_AMBIGUOUS`, `TOO_LARGE`, `MALFORMED`, `TRANSPORT`, ...). A rate-limit
message ends with `retry after <n>s` when the registry said how long to wait.
Only `transport_contract` and `malformed` catalog reads go to Sentry; outages
and throttling are logged.

## Agent tools and the `skill_setup` agent

[`tools.rs`](./tools.rs) exposes the browse/search/sources/install/uninstall operations as
LLM-callable tools (`SkillRegistryBrowseTool`, `SkillRegistrySearchTool`,
`SkillRegistrySourcesTool`, `SkillRegistryInstallTool`,
`SkillRegistryUninstallTool`), re-exported through the
`#[cfg(feature = "skills")]` glob in `crates/openhuman-core/src/tools/mod.rs`.

[`agent/skill_setup/`](./agent/skill_setup/) is a built-in agent (id `skill_setup`, delegate name
`setup_skills`) whose tool belt is the five tools above plus
`list_workflows`, `describe_workflow`, `install_workflow_from_url`,
`uninstall_workflow`, and `ask_user_clarification`. It is registered in
`crates/openhuman-core/src/agent_registry/agents/loader.rs` behind
`#[cfg(feature = "skills")]`, which embeds [`agent/skill_setup/agent.toml`](./agent/skill_setup/agent.toml) via
`include_str!` and wires `agent/skill_setup/prompt.rs::build` as its prompt
builder.

## Disabled build

When the `skills` Cargo feature is off, [`stub.rs`](./stub.rs) takes the place of this
module: the controller aggregators return empty vectors and
`ops::start_boot_catalog_refresh` is a no-op, so the always-on call sites in
`core/all.rs` and `core/runtime/services.rs` keep compiling without the real
implementation.

Default catalog:

```text
https://hermes-agent.nousresearch.com/docs/api/skills.json
```

Environment overrides for prod scripts and deterministic tests:

| Variable | Effect |
| --- | --- |
| `OPENHUMAN_SKILL_REGISTRY_CATALOG_URL` | Read the Hermes index from this URL |
| `OPENHUMAN_SKILL_REGISTRY_DOWNLOAD_BASE_URL` | Serve every `SKILL.md` from `<base>/<name>/SKILL.md` |
| `OPENHUMAN_SKILL_REGISTRY_CACHE_DIR` | Cache directory (default `~/.openhuman/skill-registry`): the legacy `hermes.json` to import and, with no backend installed, the workspace holding the default backend's `storage/` |
| `OPENHUMAN_SKILL_REGISTRY_REFRESH_ON_BOOT=0` | Skip the startup warm-up |
| `OPENHUMAN_SKILL_INSTALL_ALLOW_LOCAL_HTTP=1` | Allow plain `http` to loopback hosts (fixtures, local mirrors) |

By default, core startup spawns a background task that warms the registry
without blocking core readiness. The pre-registry `cache.json` in the store
directory is removed on first use.

Production smoke examples:

```bash
openhuman-core skill_registry schemas
openhuman-core skill_registry browse --force_refresh true
openhuman-core skill_registry search --query git
openhuman-core skill_registry sources
openhuman-core skill_registry install --entry_id git-helper
openhuman-core skill_registry uninstall --name git-helper
```

Security notes:

- Catalog and pasted-URL installs fetch `SKILL.md` through the registry's
  guarded fetch (`tinyskills::fetch_skill_document`): HTTPS only (loopback
  `http` only with `OPENHUMAN_SKILL_INSTALL_ALLOW_LOCAL_HTTP=1`), non-public
  addresses refused after the guard's own DNS resolution, connections pinned to
  the checked addresses, every redirect hop re-validated, a size cap and a
  timeout.
- The fetched document is then validated and written by
  `skills::ops_install::install_validated_document`, the same path a pasted URL
  takes; uninstall goes through `skills::ops_install::uninstall_workflow`.
- Every fetched document is scanned (`tinyskills::scan_skill`). A blocking
  scan or a failed fetch is fetched and scanned once more
  (`skills::ops_install::scan_gate`); a document that still blocks is not
  installed and the caller gets `status: "scan_blocked"` with the findings.
  Request refusals (unknown id, unsafe URL, portal entry, oversized body, rate
  limiting) are not retried, and the retry goes through the registry like the
  first attempt, so a catalog in its refresh cooldown is not refetched.
- Only `acknowledged_digest` on the `skill_registry_install` and
  `skills_install_from_url` RPCs installs a blocked document, and only the
  document with that digest. The install fetches again; if the document has
  changed, its digest no longer matches and it is scanned like an
  unacknowledged one: refused afresh with its own findings and digest, or
  installed if it now scans clean. The agent tools (`skill_registry_install`,
  `install_workflow_from_url`) neither expose the param nor return the digest,
  and always refuse; they tell the agent to send the user to the Skills page.

## Further reading

- [Parent module (`skills`)](../README.md)
- [MCP servers and skills](../../../../../docs/gitbooks/en/features/integrations/mcp-and-skills.md)
- [tinyskills submodule](../../../../../vendor/tinyskills/README.md)
- [Agent harness architecture](../../../../../docs/gitbooks/en/developing/architecture/agent-harness.md)
