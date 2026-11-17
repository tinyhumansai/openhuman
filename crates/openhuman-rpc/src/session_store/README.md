# session_store

The session store the desktop app, the CLI and the TUI install: TinyAgents'
`DriverSessionStores` over the process's storage backend, held in a
`ReboundSessionStores` (core `storage::sessions`) so a different user
signing in can move it to their workspace's backend without reinstalling the
provider.

```text
host (server shims / host::cli / host::desktop / host::tui)
  install_for_host() | provider_for_host()
     storage URL: OPENHUMAN_STORAGE_URL > [storage] url > sqlite:<workspace>/storage
     storage::open(url) -> storage::install_for_url(url, backend)
     ReboundSessionStores::new(backend, recover_on_open)
     -> storage::sessions::install(provider)        (servers: process lifetime)
     or RuntimeBuilder::session_store(provider)        (host::tui, host::cli one-shots)
```

Every agent's transcripts, turn states, records, run ledger and journal live in
that backend, one storage scope per agent. There is no file layout beside it;
what older installs wrote is imported once, read-only, by `storage::import`
when the core boots. The classic on-disk provider (`SqliteSessionStores`) is
gone.

## Install paths

- `install_for_host()` is what the server shims call, before the runtime builds
  so the recovery sweep sees the store. It does not go through
  `RuntimeBuilder::session_store`, which restores the previous provider when
  the runtime drops: the desktop restarts its in-process server in place, and a
  detached turn still writing across that gap must keep landing in the store.
- `provider_for_host()` resolves the same URL and returns the provider for a
  runtime builder (the TUI, and one-shot CLI commands that read stored state).
- `install_for_saas(url)` is the SaaS boot: with a URL the shared backend, with
  none every profile keeps its sessions in the default database of its own
  workspace (`WorkspaceSessionStores`). A backend's store never recovers on open
  in SaaS mode; recovery follows the profile lease.
- `install_for_url` / `provider_for_url` are the same with the URL already
  resolved; `None` is the SaaS case above.

A single-process backend (SQLite, memory, files) interrupts an agent's
in-flight turns the first time that agent is opened, since only an earlier
process can have left them. MongoDB does not: several processes may share the
database.

## Files

| File | Contents |
| --- | --- |
| [`mod.rs`](mod.rs) | `install_for_host`, `provider_for_host`, `install_for_url`, `provider_for_url`, `install_for_saas`, and the URL resolution. |
| [`mod_tests.rs`](mod_tests.rs) | Storage URL installation, SQLite agent isolation, environment URL resolution, an unusable URL, and the no-URL SaaS case. TinyAgents owns driver conformance tests. |
