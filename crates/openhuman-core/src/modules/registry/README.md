# modules/registry

The compiled-in table of every module this build knows how to load, split
into one file per module family.

## Why compiled in, not discovered

A loaded module is trusted native code running inside this process: it shares
the address space, the privileges, and the crash domain, and tinybus never
unloads it. Which modules may load, and which bytes count as a legitimate
copy of one, are therefore build-time decisions, not something a server could
hand the client at runtime. There is no module marketplace on purpose. A
registry a server could add entries to would be a remote-code-execution
surface with a download step attached.

## Contents

Each `records_*.rs` file defines one or more `pub(crate) const ModuleRecord`
values for a family of related modules:

- [`records_computer.rs`](./records_computer.rs): `TINYCOMPUTER` (desktop and browser control).
- [`records_docs_wallet.rs`](./records_docs_wallet.rs): `TINYDOCS`, `TINYWALLET`.
- [`records_extra.rs`](./records_extra.rs): `TINYBOX`, `TINYCHANNELS`, `TINYHOSTS`.
- [`records_mcp_connectors.rs`](./records_mcp_connectors.rs): `TINYCONNECTORS`, `TINYMCP`.
- `records_juice.rs`: `TINYJUICE`.
- [`records_search.rs`](./records_search.rs): `TINYSEARCH`.
- [`records_voice.rs`](./records_voice.rs): `TINYVOICE`.

[`../registry.rs`](../registry.rs) (one level up from this folder) wires all of them into the
`ALL` slice and answers `find(id)` by a linear scan.

## Key types

Both live in [`../types.rs`](../types.rs), not in this folder:

- `ModuleRecord`: `id`, `description`, `bus_name`, `object_path`, `version`,
  `release_url`, a `&'static [PlatformAsset]` list of per-host artifacts, and
  a `LoadPolicy` (`Lazy`, loaded on first use, or `Eager`, loaded at boot).
- `PlatformAsset`: one published artifact per host, `host_key` (for example
  `ubuntu-24.04-x86_64`), the exact release `archive` name, and its lowercase
  hex SHA-256 `sha256` digest.

Every record in this folder pins one version and one digest per supported
host, taken verbatim from that release's own `checksum.toml`. tinybus fetches
the release's `checksum.toml` itself, compares it against the digest pinned
here, hashes the downloaded archive, and only then extracts and loads. The
digest in this folder is this host's half of that agreement: pinning it in
source is what makes the check auditable by reading this file against the
release page. A digest recomputed from a local build would agree with itself
no matter what a release actually served, which is why the module doc comment
on [`../registry.rs`](../registry.rs) says not to do that.

## How it fits

`ModuleRecord::id` is the stable identifier config, RPC, and
`ensure_loaded`/`ensure_language` use to name a module. `registry::find`
is the lookup every load path goes through before the ABI, manifest, and
dependency admission checks in [`../host.rs`](../host.rs) run.

## Where next

- [`modules/README.md`](../README.md) for the full load path this registry
  feeds into (resolution order, download, digest check, `dlopen`).
- [`../types.rs`](../types.rs) for the `ModuleRecord` / `PlatformAsset` / `LoadPolicy`
  definitions this folder's records are built from.
- [`../host.rs`](../host.rs) for the broker that actually resolves, downloads, verifies,
  and loads a record this registry returns.

## Further reading

- [Parent module (`modules`)](../README.md)
- [Loadable modules](../../../../../gitbooks/developing/loadable-modules.md)
- [tinybus submodule](../../../../../vendor/tinybus/README.md)
- [Architecture overview](../../../../../gitbooks/developing/architecture.md)
