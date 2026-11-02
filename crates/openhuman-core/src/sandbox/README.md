# Sandbox host adapter

OpenHuman owns command-execution orchestration and the existing `sandbox.*`
RPC surface. TinySecurity owns sandbox planning, admission and single-use plan
consumption. TinyBox owns native jail facts and process execution.

The host builds a plan request from trusted command, environment, runtime and
caller facts, asks TinySecurity for a plan, then passes the admitted artifact to
the selected TinyBox executor. A missing module, refused plan or failed
consumption denies execution. OpenHuman maps the module results to the existing
`None`, `Local`, `Docker` or `OpenShell` routing and status contracts; it does
not duplicate the protected-path or credential-root policy.

`[security.sandbox] backend = "openshell"` routes a command to the tenant's own
long-lived box on an NVIDIA OpenShell gateway ([`openshell/`](./openshell/mod.rs)),
declared to TinySecurity as the `tinybox-openshell` container executor. Nothing
on the host is mounted, egress is denied, and the box comes from
`[runtime.openshell]`. `BoxRegistry` keys boxes on the whole `Tenant` (profile
and agent), creates each one once under concurrent first commands, reaps idle
boxes and caps live ones; a command with no tenant (`NoTenant`) is refused,
never run in a shared box.

The host keeps the `SANDBOX_ENV_PASSTHROUGH` allowlist, configuration switches,
status RPC, and executor lifecycle. The sandbox cannot be disabled for SaaS.
Local grants for stdio MCP servers use the same TinySecurity floor and TinyBox
admission path.

See [Privacy and security](../../../../docs/gitbooks/en/features/privacy-and-security.md)
for the product boundary. Policy behavior belongs in TinySecurity and native
execution behavior belongs in TinyBox; OpenHuman tests only its mapping,
routing, callbacks and module seams.
