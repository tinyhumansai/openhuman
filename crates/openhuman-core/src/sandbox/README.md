# Sandbox host adapter

OpenHuman owns command-execution orchestration and the existing `sandbox.*`
RPC surface. TinySecurity owns sandbox planning, admission and single-use plan
consumption. TinyBox owns native jail facts and process execution.

The host builds a plan request from trusted command, environment, runtime and
caller facts, asks TinySecurity for a plan, then passes the admitted artifact to
the selected TinyBox executor. A missing module, refused plan or failed
consumption denies execution. OpenHuman maps the module results to the existing
`None`, `Local`, `Docker`, `OpenShell` or `Agent37` routing and status contracts; it does
not duplicate the protected-path or credential-root policy.

`[security.sandbox] backend = "openshell"` routes a command to the tenant's own
long-lived box on an NVIDIA OpenShell gateway ([`openshell/`](./openshell/mod.rs)),
declared to TinySecurity as the `tinybox-openshell` container executor. Nothing
on the host is mounted, egress is denied, and the box comes from
`[runtime.openshell]`. `BoxRegistry` keys boxes on the whole `Tenant` (profile
and agent) within an `OwnerScope` (a hash of the active workspace, which is
per signed-in user, and on agent37 of `[runtime.agent37] retention_namespace`),
creates each one once under concurrent first commands, reaps idle boxes and
caps live ones; a command with no tenant (`NoTenant`) is refused, never run in
a shared box. A command whose caller gives up (its future dropped) lets go of
its box on a detached task, which stops the command; a create cancelled
before the module answers closes its reservation.

`[security.sandbox] backend = "agent37"` goes through the same registry
([`boxes/`](./boxes/mod.rs), `BoxBackend::Agent37`) on agent37's hosted
service, declared as the `tinybox-agent37` container executor and refused
under `[privacy] mode = "local_only"`. Its boxes are *retained* (box contract
2.4): each is created with `Retention { key, idle_sleep_secs,
delete_after_secs }` (the box contract's own `tinybox_bus::Retention`), where
`key` is `boxes::owner_key`, `oh-` plus the SHA-256 of the owner scope and the
whole tenant (stable, opaque, never a raw id or path). Deployments sharing one
agent37 key that could see equal tenants in equal workspace paths (staging and
production of one service) must set distinct `retention_namespace` values;
changing it starts each tenant on a fresh box and the old one expires. The service
sleeps a box after `[runtime.agent37] idle_sleep_secs` (default 900, clamped
to 300–86400), wakes it on the next command, and deletes it after
`delete_after_secs` asleep (default 30 days). The registry never deletes a
retained box: reaping, `release`, `shutdown`, a box that stopped answering and
a full registry (which lets go of its least recently used idle box) all only
forget it (`BoxDriver::forget`, the module's `Close`, which keeps a retained
instance). The next create with the same key adopts it, files included, even
after a restart. OpenShell boxes are not retained and are closed as before;
TinyBox refuses retention on every backend without `Capability::Retention`.

## Files in and out of a box

A box has its own disk: a user's upload on the host is not in it, and a file the
agent makes in it is not on the host. `boxes::write_file` and
`boxes::read_file` (re-exported as `sandbox::write_box_file` /
`read_box_file`, and as `openhuman_embed::sandbox::{write_file, read_file}`)
move bytes across over plain command execution (`BoxDriver::exec`), so any box
driver supports them and no box contract grows. The generic forms
`write_file_in` / `read_file_in` take the registry, for tests and drivers.

- Paths are relative to the box work folder (`BoxBackend::workdir`: `/sandbox`
  on OpenShell, `/home/node` on agent37). The process-level calls use the
  configured backend and refuse one that keeps no boxes (`NoBoxBackend`). Absolute paths, `..`, NUL and empty paths are refused; every path
  that reaches the shell is single-quoted and written `./<path>`. Every command
  resolves the path inside the box and refuses one that leaves the work folder
  (a symlink out of it included). Files over `MAX_FILE_BYTES` (8 MiB)
  are refused, and a read also honours the caller's `max_bytes`.
- Write: 48 KiB raw per command (`printf %s '<base64>' | base64 -d >> <part>`;
  a Linux argument is at most 128 KiB). The part file is beside the target, named
  uniquely per transfer and created exclusively, so an existing file or a
  concurrent upload is never overwritten. The last command checks the part file's
  length and renames it over the target, so a box replaced mid-write fails
  instead of keeping a truncated file. A failure removes the part file.
- Read: `test -f` and `stat` (size and identity) first, a file over the limit is refused before any
  chunk moves, then 256 KiB raw per `tail | head | base64 -w0` (a hosted exec
  keeps 512 KB of output per stream). The identity is checked before and after
  every chunk, so a file replaced mid-read fails. A short read is an error.
- Failures are typed `BoxError`s (`InvalidPath`, `FileTooLarge`, `FileNotFound`,
  `ShortRead`, `FileTransfer`) whose messages read as a tool result; none names a
  host path.
- Keyed on the whole tenant like a command; no tenant (`NoTenant`) is refused.

`create_artifact` with a file `path` uses this when the backend keeps boxes
(`SandboxBackendKind::keeps_boxes`: `openshell` and `agent37`): `artifacts::box_source`
reads the file from the calling tenant's box, stages it under the artifacts
kept-copies folder for the call, and hands it to the artifact service like any
file. Other backends read the host disk as before.

The host keeps the `SANDBOX_ENV_PASSTHROUGH` allowlist, configuration switches,
status RPC, and executor lifecycle. The sandbox cannot be disabled for SaaS.
Local grants for stdio MCP servers use the same TinySecurity floor and TinyBox
admission path.

See [Privacy and security](../../../../docs/gitbooks/en/features/privacy-and-security.md)
for the product boundary. Policy behavior belongs in TinySecurity and native
execution behavior belongs in TinyBox; OpenHuman tests only its mapping,
routing, callbacks and module seams.
