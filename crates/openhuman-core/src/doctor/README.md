# Doctor

The doctor domain runs configuration and runtime diagnostics and returns a
versioned `DoctorReport`. Checks have stable ids, a cost tier, a finding for
every result, and a fix hint. The core owns the report contract, check registry,
runner, core checks, and JSON-RPC controllers. Host-specific checks register
through the same source interface.

## Contract

`DoctorReport.schema_version` is `2`. A report carries the selected profile
and config source when known, the requested tier, ordered `checks_run` and
`checks_skipped` ids, findings, a severity summary, and `ok`. `ok` is false when
at least one finding has severity `error`; warnings and skipped checks do not
make it false.

Each `Finding` contains its check id, severity (`pass`, `info`, `warn`, `error`,
or `skipped`), safe message, required `fix_hint`, optional config path and
value source, elapsed milliseconds, and affected capability ids. Findings must
not include credentials, resolved config values, request bodies, or user text.
Use a safe error class and a useful recovery step when the original error may
contain sensitive data. A passing check uses “No action required.” as its hint.

## Tiers and selection

| Tier | Runs | Budget |
| --- | --- | --- |
| `static` | Local config and filesystem checks. | 5 seconds per check. |
| `connect` | Static checks and bounded, read-only connectivity probes. This is the default. | 10 seconds per check. |
| `live` | Static and connect checks plus probes that may incur inference cost or write disposable remote test data. | 60 seconds per check. |

The runner allows at most eight checks at once. Synchronous checks run on
Tokio's blocking pool; async checks stay on the runtime. A timeout or failed
worker becomes an error finding and does not stop sibling checks. Tier and
component gates produce `skipped` findings before a detector runs.

`only` and `skip` accept exact registered ids. When `only` is present, checks
outside that list do not appear in the report. `skip` removes checks from the
selected set. Unknown ids add one `doctor/selection` error finding without
echoing the supplied value. Check ids use lowercase slash-separated segments;
duplicate and malformed registrations are rejected before any detector runs.

## Registry and check sources

Built-in checks are composed in `checks/`, with profile checks in `profile.rs`.
Configuration-dependent checks are selected for the report's `DoctorCtx`.
Hosts contribute additional checks through `DoctorCheckSource`:

- `register_check_source` adds a source for the process.
- `register_check_source_in` scopes a source to a `CoreContext` extension set.
- `checks_for(&DoctorCtx)` lets a source choose checks from the effective
  config and profile; context-independent sources may use the default method.

Use a process source for checks independent of the active user or profile. Use
context-scoped registration and the supplied `DoctorCtx` for checks whose ids
or checks depend on that context. `registered_checks_for` combines built-ins,
context-dependent checks, and active host sources, then validates the complete
set before the runner starts.

## Profiles and hosted callers

The active config supplies the default report. A desktop host may select a
profile with `profile`; the doctor loads that profile read-only and reports
selection failures as findings. It does not activate or repair the profile. A
SaaS caller always uses its current tenant context and cannot use the profile
input to inspect another tenant. Hosts should route selected-profile requests
through `doctor_report_for_dispatch` or the explicit profile resolver so the
core uses the host's authorization and profile rules.

## Controller and tool surfaces

The `doctor.report` JSON-RPC controller accepts optional `tier`, exact-id
`only` and `skip` arrays, and `profile`. It returns the v2 report. The CLI host
is a separate consumer of this controller; CLI argument parsing, presentation,
and exit behavior belong in `crates/openhuman-cli`, not in this domain.

The `doctor_health` agent tool accepts optional `tier` and `only`. It defaults
to `connect` and returns the same report contract. Its description tells the
model that `live` may spend inference tokens and is only appropriate when the
user explicitly requested those probes; the tool has no separate user-intent
signal to enforce that policy at execution time.

The `doctor.models` controller and `doctor_models` tool are separate surfaces.
Keep their wire names stable. Catalog inspection uses a dedicated hook that
freezes selected credentials and never falls back to the ordinary mutable
catalog callback.

## Key files

| File | Role |
| --- | --- |
| [`contract.rs`](./contract.rs) | Tiers, check context and source traits, findings, and report types. |
| [`registry.rs`](./registry.rs) | Built-in and host source composition, registration, and id validation. |
| [`runner.rs`](./runner.rs) | Selection, tier/component skips, bounded execution, and report assembly. |
| [`checks/`](./checks/) | Configuration, inference, memory, storage, and context-selected checks. |
| [`profile.rs`](./profile.rs) | Read-only desktop profile loading and profile diagnostics. |
| [`ops.rs`](./ops.rs) | Core report entry points and host-facing profile selection. |
| [`schemas.rs`](./schemas.rs) | JSON-RPC controller schemas and handlers. |
| [`tools.rs`](./tools.rs) | `doctor_health` and `doctor_models` agent tools. |

## Tests

Tests live beside the implementation in `*_tests.rs` files. Use
`test_support::FakeCheck` and context-scoped check sources for runner and tool
tests. Keep tests independent of user skill roots and external services. The
registered-id inventory test under `tools_tests.rs` checks the user guide
against the check registry so documented ids track the code.
