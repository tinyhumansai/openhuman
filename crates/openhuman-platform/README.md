# openhuman-platform

The OS-facing pieces of the OpenHuman core that need no product
configuration: the per-user service managers that install and drive the core
daemon, and the GitHub-release staging the self-update path downloads through.

It is a leaf below the core in the crate chain
([`scripts/ci/check-crate-chain.mjs`](../../scripts/ci/check-crate-chain.mjs)):
it names no OpenHuman crate, and knows nothing of `Config`, the event bus
(`DomainEvent`) or the RPC envelope (`Outcome`). The core describes what to
install and where releases live, and keeps everything product-shaped.

```text
 openhuman-core  platform::service   controllers, schemas, restart/shutdown,
       |                               daemon-host prefs, tools, spec.rs
       |         platform::update    controllers, policy + URL/asset allowlist,
       |                               scheduler, tools, source.rs
       v
 openhuman-platform
   service::{install, start, stop, status, uninstall}(&ServiceSpec)
   update::{check_available, download_and_stage}(&UpdateSource, .., &dyn UpdateReporter)
```

## `service`

`ServiceSpec { label, legacy_labels, exe, args, logs_dir, work_dir }` names the
service for the current OS. The core builds it in
`crates/openhuman-core/src/platform/service/spec.rs` (labels
`com.openhuman.core` / `openhuman.service` / `OpenHuman Core`, the daemon
executable lookup, `["run"]`, `<config dir>/logs`).

| File | What it does |
| --- | --- |
| [`src/service/mod.rs`](src/service/mod.rs) | `ServiceSpec`, `ServiceState`, `ServiceStatus`, and the dispatchers that route to the mock or the per-OS manager. |
| [`src/service/macos.rs`](src/service/macos.rs) | LaunchAgent plist in `~/Library/LaunchAgents`, `launchctl bootstrap/kickstart/bootout`; also stops and removes the spec's legacy labels. |
| [`src/service/linux.rs`](src/service/linux.rs) | systemd user unit under `<work_dir>/.config/systemd/user/`, `systemctl --user`. |
| [`src/service/windows.rs`](src/service/windows.rs) | Scheduled task (`schtasks`, on logon) running a `.cmd` wrapper in `logs_dir`. |
| [`src/service/mock.rs`](src/service/mock.rs) | File-backed mock for E2E, on when `OPENHUMAN_SERVICE_MOCK` is truthy; state in `OPENHUMAN_SERVICE_MOCK_STATE_FILE` or `<work_dir>/service-mock-state.json`, with forced failures and the `agent_running` flag (`mock_agent_running`). |
| [`src/service/common.rs`](src/service/common.rs) | Command runners (Windows `CREATE_NO_WINDOW`), `xml_escape`, the unit command line. |

## `update`

`UpdateSource { owner, repo, asset_prefix }` names a release stream; its
fields are `&'static str` because the release origin must be compiled in, never
runtime-configurable. `UpdateReporter` is the error-report callback: the core's
implementation sends `report` to Sentry under the `update` domain and supplies
the updater's transient-failure classifiers.

| File | What it does |
| --- | --- |
| [`src/update/release.rs`](src/update/release.rs) | `check_available` (GitHub `releases/latest`, semver compare, platform asset match for `<prefix>-<version>-<triple>.tar.gz`/`.zip` or the legacy raw binary) and `download_and_stage` (checksum first, stream, verify, extract the inner `<prefix>` binary, atomic rename to `<prefix>.staged`), `platform_triple`. |
| [`src/update/download.rs`](src/update/download.rs) | The verified download: `.sha256` sidecar fetch (fails closed), declared-size and 512 MiB caps, streaming SHA-256. |
| [`src/update/types.rs`](src/update/types.rs) | `UpdateSource`, `UpdateReporter`, `UpdateInfo`, `StagedUpdate`, the GitHub release shapes. |

Which callers may trigger an update (`config.update.rpc_mutations_enabled`),
the download-URL host allowlist and asset-name check, the restart strategy and
the periodic checker are the core's (`crates/openhuman-core/src/platform/update/`).

## Tests

```bash
cargo test -p openhuman-platform
```

Tests live in sibling `*_tests.rs` files. The release and download suites run
against a local `wiremock` server; the mock-service suite serializes on one env
lock (`src/test_env.rs`). The macOS, Linux and Windows managers are compiled
only on their own OS.
