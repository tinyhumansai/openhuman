# gateway

Routes the frontend's JSON-RPC to a core wherever that core runs. The default
is the core embedded in this process. A gateway can also point at a core
someone else runs (a URL), or at one this app provisions itself: in a Docker
container on this machine, as a process on another machine over SSH, or in a
container on another machine over SSH. The module is compiled only with the
`gateways` Cargo feature, which is on by default.

## How it works

The design rests on one seam: a gateway resolves to a URL and a bearer, and
nothing else in the app changes. The `core_rpc_endpoint`, `core_rpc_url` and
`core_rpc_token` commands in `lib.rs` answer from
`registry::current`, so `coreRpcClient`, `relay_http_rpc` and every screen
reach a remote core through the same code that reaches the embedded one. No
transport abstraction is threaded through the frontend.

```text
renderer                        shell
--------                        -----
gateway_save(Gateway) --------> store::save -> <data dir>/gateways.json
gateway_activate(id) ---------> registry::activate
                                   |  holds ACTIVATION_LOCK for the whole call
                                   v
                                ops::activate(spec)
                                   |
             +---------------------+----------------------+
             | Desktop             | Remote               | Box
             v                     v                      v
      desktop.ensure_running   nothing to do       provision::provision
             |                     |                      |
             +--> ops::endpoint_of +           Provisioned { active, forward,
                  (url, token)                  sandbox, box_id, process }
                                   |
                                   v
                     STATE.active = ActiveGateway { id, rpc_url, token }
                     previous Provisioned -> tear_down()
                                   |
core_rpc_endpoint ----------------> registry::current -> (rpc_url, token)
```

### Gateway kinds

`types::GatewaySpec` has three variants:

- `Desktop` is the embedded core. A built-in record with id `desktop`
  (`Gateway::desktop`, label "This computer") is always listed first and
  cannot be replaced or deleted.
- `Remote { url, token }` is a core someone else runs. Nothing is provisioned;
  the URL is the answer.
- `Box { reach, confinement, env }` is a core this app starts with tinybox.
  `Reach` is `Local` or `Ssh(SshReach)` (destination, optional port, optional
  identity file, `accept_new_host_key`). `Confinement` is `Passthrough
  { binary, workspace }` (an ordinary process) or `Docker { image }`. The two
  axes are independent, so SSH plus Docker needs no variant of its own.
  tinybox's `namespace` and `microvm` sandboxes are not offered because they
  cannot keep a detached server running between commands.

`GatewaySpec::kind` gives a short stable word for logs and the picker badge:
`desktop`, `remote`, `docker`, `ssh`, `ssh+docker` or `local-process`.

### Provisioning a box

`provision::provision` loads the persisted runtime config through the curated
embed facade and calls the pinned TinyBox module before the core RPC server is
started. The host reserves opaque resource, process, and forward ids before
native work, then sends typed requests through `openhuman_rpc::embed::modules::tinybox`.

The gateway chooses the host (`Local` or configured `Ssh`), workspace
(directory or OCI image), egress policy, and process environment. TinyBox owns
creation, the detached core process, and its forward. Docker requests a dynamic
host port; TinyBox returns the effective mapping and owns the forward rather
than relying on shell-side Docker probing or port-conflict parsing. The core
runs as `<binary> serve` (or `openhuman-core serve` from the image's `PATH`) with
a freshly minted `OPENHUMAN_CORE_TOKEN`, `OPENHUMAN_CORE_HOST=0.0.0.0`, and
`OPENHUMAN_CORE_PORT=7788`.

After opening the module-owned forward, the gateway polls the unauthenticated
`/health` endpoint every 250 ms for up to 120 seconds. A failure closes the
caller-known resource reservation, which lets the module clean up even if a
Create/Spawn/Forward reply was lost after native startup. The resulting
`Provisioned` value retains the explicit config and resource handle; teardown
uses TinyBox `Close` and logs a sanitized error if native cleanup remains pending.

### Activation and switching

`registry::activate` serializes whole activations with `ACTIVATION_LOCK`, so
the last activation to start is also the last to finish. A generation counter
stops a late progress update from a superseded activation from overwriting a
terminal `Connected` or `Failed` status.

The previous gateway is torn down only after the new one is up. A failed
activation records `GatewayStatus::Failed { reason }` and leaves the working
gateway active. `Provisioned::tear_down` asks TinyBox to close the resource and waits for its
cleanup acknowledgement; the module retains unacknowledged native ownership.

At launch, and whenever nothing has been activated, `registry::current` falls
back to the embedded core. Active state is in memory only; after a restart
the app starts on the desktop gateway until the frontend activates another.
On exit, `lib.rs` calls `registry::shutdown`, which tears down any provisioned
box.

## Layout

| File | What it does |
| --- | --- |
| [`mod.rs`](mod.rs) | Module declarations and the design notes. No re-exports. |
| [`types.rs`](types.rs) | `Gateway`, `GatewaySpec`, `Reach`, `SshReach`, `Confinement`, `ActiveGateway`, `GatewaySummary`, `GatewayStatus`, and `validate_remote_transport`. |
| [`store.rs`](store.rs) | Reads and writes `gateways.json` in the shell data dir (`file_logging::resolve_data_dir`). |
| [`ops.rs`](ops.rs) | `activate` decides what a spec needs; `endpoint_of` answers for non-provisioning specs; `Provisioned` and its `tear_down`. |
| [`provision.rs`](provision.rs) | The four tinybox steps, port selection and cleanup on failure. |
| [`registry.rs`](registry.rs) | The active gateway, its held-open `Provisioned`, status and the activation lock. |
| [`commands.rs`](commands.rs) | The Tauri commands. Each one resolves arguments and delegates to `store` or `registry`. |

## Key types and entry points

- `GatewaySpec` ([`types.rs`](types.rs)) is what the user configured.
- `ActiveGateway` (`types.rs`) is what activation produces: `id`, `rpc_url`,
  optional `token`. It never leaves the shell.
- `GatewaySummary` (`types.rs`) is the credential-free view the renderer gets
  from `gateway_list`: id, label and kind.
- `registry::current` ([`registry.rs`](registry.rs)) is called by `active_rpc_endpoint` in
  `lib.rs` on every endpoint lookup.
- `registry::activate` (`registry.rs`) is the only writer of the active state.

## Tauri commands

| Command | Behavior |
| --- | --- |
| `gateway_list` | All gateways as `GatewaySummary`, desktop first. |
| `gateway_save` | Adds or replaces a record. Does not activate. |
| `gateway_delete` | Removes a record. Deleting the active gateway does not switch away from it. |
| `gateway_activate` | Activates by id and returns nothing; the bearer stays in the shell. |
| `gateway_active` | The active gateway's id. |
| `gateway_status` | `inactive`, `activating { step }`, `connected { endpoint }` or `failed { reason }`. |

## Boundaries

- Box lifecycle, SSH reach, Docker confinement and port forwarding are
  implemented in the [`vendor/tinybox`](../../../../vendor/tinybox/) submodule (`tinyhumansai/tinybox`,
  crates `tinybox-core`, `tinybox-host`, `tinybox-ssh`, `tinybox-docker`).
  Fix behavior there, not here.
- The `openhuman-core serve` command, its bearer check and `/health` belong to
  the core and [`crates/openhuman-rpc`](../../../openhuman-rpc/).
- The frontend's gateway picker (`GatewaySection`) and its `coreMode` setting
  live in [`app/src`](../../../../app/src/).

## Gotchas

- Credentials stay in the shell. Gateway records live in `gateways.json`, not
  renderer `localStorage`, because a renderer XSS can read anything stored
  there. `gateway_list` strips the bearer, SSH destination and identity path,
  and `gateway_activate` does not return the endpoint.
- A `Remote` gateway with a bearer must use `https` unless the host is
  loopback. `store::save` enforces this through `validate_remote_transport`,
  and `core_rpc::post_json_rpc` checks it again before sending.
- A box's bearer is minted per activation and passed only as an environment
  variable, so a stored record never holds a credential for a running core.
- `store::list` returns just the desktop gateway when the file is missing or
  unreadable, while `store::save` refuses to overwrite a file it cannot parse.
- Shell-side callers that use `core_rpc::core_rpc_url_value` (the iMessage
  scanner) still reach the embedded core while a gateway is active.

## Tests

[`ops_tests.rs`](ops_tests.rs), [`registry_tests.rs`](registry_tests.rs), [`store_tests.rs`](store_tests.rs) and [`types_tests.rs`](types_tests.rs) sit
in this directory and are declared from [`mod.rs`](mod.rs).

```bash
cargo test --manifest-path crates/openhuman-app/Cargo.toml gateway::
```

## Further reading

- [`docs/gitbooks/en/developing/architecture/tauri-shell.md`](../../../../docs/gitbooks/en/developing/architecture/tauri-shell.md): the Tauri shell.
- [`crates/openhuman-app/README.md`](../../README.md): the openhuman-app crate README.
- [`crates/openhuman-rpc/README.md`](../../../openhuman-rpc/README.md): the openhuman-rpc crate README.
