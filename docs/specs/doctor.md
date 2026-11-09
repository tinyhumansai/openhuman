# Doctor v2 contract

This is the maintainer contract for the OpenHuman doctor framework and the
checks delivered by the doctor epic. It defines stable check ids and report
semantics for the `doctor` domain. The core owns checks,
the registry, runner, and RPC surface; `openhuman-cli` owns command parsing,
rendering, and process exit codes; `openhuman-tinyhumans` owns hosted checks.

The intended user outcome is a report that says whether the selected
configuration is ready to run, what was inspected, and what an operator can do
about each finding. Doctor does not persist configuration or application-state
changes; local filesystem write probes use temporary files and clean them up.
This contract borrows OpenClaw's stable check identifiers,
structured findings, advisory JSON mode, and threshold-based lint behavior.
Wire fields remain snake case to match the existing RPC contract. This contract
does not adopt OpenClaw's repair operation: this epic adds no repair command.

## Goals and non-goals

The doctor validates configuration semantics and provenance, profile-specific
runtime readiness, configured local and hosted components, and outbound
connections. Checks are selectable by stable id and have an explicit cost tier.
The report is a versioned JSON API shared by RPC, CLI, the agent tool, and the
Settings diagnostics surface.

This epic does not add `--fix`, prompts, config rewrites, migrations, token
generation, or other repair side effects. A finding always has a human-readable
`fix_hint`, but that hint is guidance for a later explicit operator action.
Checks do not make billable inference or embedding calls, or write scratch
memory, unless the caller opts into the `live` tier.

## Check contract

The Rust API and serialized JSON use snake case. A check is registered with a
stable id, category, cost tier, and optional component gate. Its detector is
asynchronous and returns findings for that check:

```rust
DoctorCheck {
    id: "<area>/<name>[/<qualifier>...]",
    category: "<display category>",
    tier: DoctorTier::Static | DoctorTier::Connect | DoctorTier::Live,
    component_gate: Option<ComponentId>,
    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding>,
}
```

Ids are lowercase ASCII segments separated by `/`, with at least an area and
name (`area/name`). Each segment is nonempty and contains only lowercase ASCII
letters, digits, `_`, or `-`. Additional segments are allowed for checks
scoped to a role or provider, such as
`inference/live/chat`. They are API identifiers used by `--only`, `--skip`, the
RPC filters, and downstream consumers. Once released, an id is never renamed
or reused for different behavior; a replacement gets a new id and the old one
remains recognizable for at least one schema lifetime. IDs are matched
exactly. Prefixes, globs, and category names are not selectors.

`static` checks may read and probe the local filesystem, but do not open network
connections. Temporary local write probes must be cleaned up. `connect` checks
may make bounded, read-only network requests and are the default; they do not
make billable calls or persist state. `live` checks may make billable model or
embedding calls and may write disposable scratch memory; they run only when
explicitly requested. A requested tier is a ceiling: `static` runs static
checks, `connect` runs static and connect checks, and `live` runs all three
tiers.

The runner applies the active `ComponentSet` gate before calling a detector.
A gated-off component produces a `skipped` finding and is listed in
`checks_skipped`. Checks above the requested tier are also listed as skipped.
An id omitted by `--only` or named by a valid `--skip` is filtered out and is
not reported as a failed check. The runner uses bounded concurrency (at most
eight checks at once), a timeout for every detector (5 seconds for static,
10 seconds for connect, 60 seconds for live), and a stable registry order in
the report regardless of completion order. Timeout and detector failures are
`error` findings; one failure does not prevent other selected checks from
finishing. A timed-out detector is cancelled where the async operation supports
cancellation.

Detectors may return an empty vector when the check is clean. The runner turns
an empty result into one `pass` finding so `checks_run` and severity totals
still distinguish a successful check from an unobserved one. Detectors that
find multiple conditions emit one finding for each condition. This permits a
clean config check to return no findings, as required by the config leaves.

### Finding

```json
{
  "check_id": "config/unknown-key",
  "severity": "warn",
  "message": "Unknown config key `memroy`; did you mean `memory`?",
  "fix_hint": "Rename the key to `memory` and rerun doctor.",
  "config_path": "memroy",
  "value_source": "file",
  "duration_ms": 4,
  "affects": ["settings.configure_ai"]
}
```

| Field          | Contract                                                                                                                                                        |
| -------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `check_id`     | Required stable id of the check that produced the finding.                                                                                                      |
| `severity`     | `pass`, `info`, `warn`, `error`, or `skipped`.                                                                                                                  |
| `message`      | Short operator-facing result, with no secret or config value.                                                                                                   |
| `fix_hint`     | Required next step. Use “No action required.” for `pass`; explain how to rerun or enable a skipped check.                                                       |
| `config_path`  | Optional config key path or source file path. `config/source` may name the loaded config and `.env` paths; other checks normally use a logical config key path. |
| `value_source` | Optional `cli`, `env`, `dotenv`, `file`, or `default`, when a finding concerns a resolved value.                                                                |
| `duration_ms`  | Elapsed detector time, repeated consistently on findings from one check. A check skipped before execution reports `0`.                                          |
| `affects`      | Zero or more stable `about_app` capability ids, such as `channels.connect_platform` or `local_ai.configure_provider`.                                           |

The report must not contain credentials, tokens, secret material, raw config
values, request or response bodies, or user-authored content. Redact endpoint
credentials and query parameters before forming messages. A detector that
cannot safely summarize an error reports its class and a safe recovery hint,
not the original error string. `value_source` records provenance only; it
never carries the value.

## Report schema v2

The RPC method remains `openhuman.doctor_report`; the existing `doctor_models`
method and `doctor_health` / `doctor_models` tool names also remain stable.
`doctor_report` accepts `{ "tier", "only", "skip", "profile" }` and returns:

```json
{
  "schema_version": 2,
  "profile": "desktop-default",
  "config_source": "profile",
  "profile_layout_version": 2,
  "config_resolution_source": "active_user.toml",
  "tier": "connect",
  "checks_run": ["config/parse"],
  "checks_skipped": ["inference/live/chat"],
  "findings": [
    {
      "check_id": "config/parse",
      "severity": "pass",
      "message": "The config file parsed successfully.",
      "fix_hint": "No action required.",
      "duration_ms": 4,
      "affects": []
    },
    {
      "check_id": "inference/live/chat",
      "severity": "skipped",
      "message": "This live check was not requested.",
      "fix_hint": "Rerun doctor with --live to run this check.",
      "duration_ms": 0,
      "affects": []
    }
  ],
  "summary": { "pass": 1, "info": 0, "warn": 0, "error": 0, "skipped": 1 },
  "ok": true
}
```

`checks_run` and `checks_skipped` are ordered arrays of check ids. A check
selected by the caller but gated off or above the requested tier is in
`checks_skipped`; checks outside an `only` selection and checks named by `skip`
are absent. `summary` counts findings by severity. `ok` is false exactly when
the report contains an `error`; warnings remain advisory, and a skipped check
does not by itself make the report unhealthy. The CLI's lint threshold is
separate from this field.

`profile` is the effective profile id, when the host has one, and `null`
otherwise. `config_source` is always the source kind (`profile` or `tenant`),
not a tenant credential or filesystem path. `profile_layout_version` is `1`
or `2` when a desktop profile layout was selected; `config_resolution_source`
is the stable directory-selection label, such as `OPENHUMAN_WORKSPACE`,
`active_workspace.toml`, `active_user.toml`, `default`, or `--profile`. These
optional fields are omitted when unknown and for SaaS tenant contexts. The
`config/source` info finding
reports the loaded config and `.env` paths, directory resolution source,
profile layout version when applicable, and whether CLI `--model` or
`--provider` overrides are active. It does not report config values or
credentials. It also reports `.env` variables shadowed by existing process
environment values without including their values.

The selection validator is a reserved runner check, `doctor/selection`. An
unknown id supplied to either `only` or `skip` adds an `error` finding under
`doctor/selection`, with a hint to use a registered exact id. Do not echo an
unknown selector into the message. Valid selected checks still run. The
reserved check is not itself selectable and does not count as a detector in
`checks_run`.

## Tier and selection inputs

The RPC accepts optional `tier`, `only`, `skip`, and `profile` fields. An
omitted tier means `connect`; omitted lists are empty. `only` and `skip` are
repeatable at the CLI and contain exact ids. If `only` is present, only those
registered checks run; `skip` then removes checks from that set. Duplicate ids
are collapsed. An unregistered id is a report finding rather than a transport
error, so callers receive useful results for the valid ids in the same request.

`--live` selects the `live` ceiling. It does not change the tier of each check
or imply repair. The doctor does not infer consent to billable work from a
profile setting.

## CLI contract

The command is implemented only in `crates/openhuman-cli`; the core contains
no CLI parser or renderer. It calls `openhuman.doctor_report` through the
`openhuman-rpc` facade and maps options to the RPC parameters above.

| Invocation                            | Output and exit behavior                                                                                                                    |
| ------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `openhuman-core doctor`               | Human-readable table; exit `0` after a report is produced.                                                                                  |
| `openhuman-core doctor --json`        | JSON report; advisory mode exits `0` after a report is produced, regardless of findings.                                                    |
| `openhuman-core doctor --lint`        | Human-readable findings; exit `0` if none meet the threshold, `1` if any do, `2` if parsing or execution fails before a report is produced. |
| `openhuman-core doctor --lint --json` | Same threshold and exit behavior as lint, with JSON output.                                                                                 |

`--severity-min` accepts `info`, `warn`, or `error` and defaults to `error`.
The threshold controls both printed findings and the lint result. `pass` and
`skipped` never meet it. `--only <id>` and `--skip <id>` may be repeated;
`--live` selects the live tier, and `--profile <id>` selects the effective
profile where the host permits it. In SaaS, an authenticated tenant always uses
its forced config and storage scope. An operator may use `--profile <id>` to
target another tenant; a tenant caller may target only itself. The host checks
that permission before doctor reads config or credentials. Argument syntax
errors exit `2` before
the RPC call.

The CLI does not add fields to the RPC report. Human output is a rendering of
the same findings and includes the severity, check id, message, and fix hint.
JSON uses the same report in every surface.

`--help` (also `-h` or `help`) prints usage and exits `0`. When no report is
produced the CLI exits `2` and names one fixed reason; the underlying error is
never printed, because it can carry endpoints or credential context. Text mode
prints `doctor could not produce a report: <reason>` to stderr; `--json` prints
`{"ok": false, "reason": "<reason>"}`. The reason strings are stable:

| Reason                                                                     | Meaning                               |
| -------------------------------------------------------------------------- | ------------------------------------- |
| `the async runtime could not start`                                        | The CLI could not create its runtime. |
| `the doctor host could not start (profile selection, config or core boot)` | The in-process host did not start.    |
| `the doctor_report call failed`                                            | The core rejected or failed the RPC.  |
| `the core answered without a report`                                       | The RPC returned no report object.    |

## Profile semantics

On desktop, a requested profile id resolves through `ProfileLayout`. Doctor
loads that profile's configuration and credentials for inspection without
changing the active profile, writing profile state, or changing process-wide
configuration. With no explicit id, it uses the currently selected profile.
The report's `profile` is the effective id and `config_source` is `profile`.

In SaaS, doctor runs against a forced tenant configuration and storage scope.
Without `profile`, the context is the authenticated tenant. An operator may
select another tenant by id; an ordinary tenant caller may select only itself.
The host authorizes this before doctor reads any config or credential. The
report uses `config_source: "tenant"` and includes no tenant secret.
Profile-scoped reads use the selected context rather than process-global
lookup helpers.

## Check registration and hosted checks

Built-in checks register in the core registry. The core exposes both
process-default `register_check_source(source)` and per-`CoreContext`
`register_check_source_in` registration through `CoreBuilder::extensions`, so a
host can supply checks without a dependency from core back to that host.
`openhuman-tinyhumans` registers its backend, identity, socket, API URL, and
managed-inference checks through this seam. Profile-aware or tenant-aware
sources use context-scoped registration; context-independent sources may use
the process-default registration. At report start, malformed or duplicate ids
are rejected before any selected detector executes. Hosted ids use the same
lowercase slash-separated segment grammar and are unique across sources. A
source may register zero checks when its host capability is absent.

The doctor stays in the existing `DomainGroup::Platform` registration group.
Its Rust module lives at the top-level `doctor` domain without adding a new
runtime domain group. `subsystems.status` remains a separate
wire projection: it reports driver bindings, health, and capabilities, while
doctor findings include stable check ids, severity, guidance, provenance, and
timing. A doctor check may consume subsystem status as evidence, but it does
not reuse `SubsystemStatus` as a finding.

## Check inventory

The paths below are the implementation-owned probe locations under
`crates/openhuman-core/src/doctor/` (and `crates/openhuman-tinyhumans/src/doctor/`
for hosted checks). Function names are stable implementation targets. The
implementation leaves add final source-line links when their functions land;
this contract is written before those files exist. IDs and any severity/status
called out by a leaf's Scope or Acceptance criteria are normative. Where a leaf
does not specify a severity, its implementation must choose one consistent with
the finding and cover it in that leaf's tests.

| Check id                             | Tier    | Probe function                                                  | Owning leaf | Required severity or status                                                                     | Minimum fix hint                                                              |
| ------------------------------------ | ------- | --------------------------------------------------------------- | ----------- | ----------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| `config/parse`                       | static  | `doctor/checks/config.rs::parse`                                | #492        | `error` on parse failure or corruption recovery, including `.bak` fallback.                     | Repair `config.toml`; review and restore a valid backup if recovery occurred. |
| `config/unknown-key`                 | static  | `doctor/checks/config.rs::unknown_key`                          | #492        | `warn` per ignored key, with a did-you-mean suggestion where available.                         | Rename the key to the suggested supported field.                              |
| `config/legacy-key`                  | static  | `doctor/checks/config.rs::legacy_key`                           | #492        | `info` per migrated legacy key.                                                                 | Update the config to the current key name.                                    |
| `config/env-override`                | static  | `doctor/checks/config.rs::env_override`                         | #493        | `warn` if env overrides an explicit file value; otherwise `info`.                               | Set the intended value in one source and remove the shadowed override.        |
| `config/source`                      | static  | `doctor/checks/config.rs::source`                               | #493        | `info`.                                                                                         | No action required; use the reported source when diagnosing precedence.       |
| `config/runtime-settings`            | static  | `doctor/checks/runtime_settings.rs::runtime_settings`           | #506        | `warn` for ignored fallback providers, incomplete model routes or delegate models, or no channel when channels are enabled. | Remove ignored settings or complete the reported configuration path. |
| `inference/provider-ref`             | static  | `doctor/checks/inference.rs::provider_ref`                      | #494        | `error` for unresolved provider references.                                                     | Select a registered provider and fix the reported config path.                |
| `inference/construct`                | static  | `doctor/checks/inference.rs::construct`                         | #494        | `error` when a configured role cannot be constructed, including missing BYOK credentials.       | Correct provider settings and configure its required credential.              |
| `inference/temperature`              | static  | `doctor/checks/inference.rs::temperature`                       | #494        | Finding for invalid settings; severity is not specified by the leaf.                            | Set a supported temperature for the configured role.                          |
| `embeddings/config`                  | static  | `doctor/checks/embeddings.rs::config`                           | #494        | Finding for invalid or inconsistent settings; severity is not specified by the leaf.            | Configure a provider, model, and dimension that agree.                        |
| `components/referenced-but-disabled` | static  | `doctor/checks/components.rs::referenced_but_disabled`          | #495        | `warn` when config references a disabled component.                                             | Enable the component or remove dependent configuration.                       |
| `modules/source-fallback`            | static  | `doctor/checks/modules.rs::source_fallback`                     | #495        | Leaf-defined; include the source fallback detail.                                               | Configure the intended module source explicitly.                              |
| `modules/override-invalid`           | static  | `doctor/checks/modules.rs::override_invalid`                    | #495        | Leaf-defined for an invalid override.                                                           | Correct or remove the invalid module override.                                |
| `storage/url`                        | static  | `doctor/checks/storage.rs::url`                                 | #495        | `error` when the storage scheme's driver is unavailable in this build.                          | Use a supported URL scheme and enable its driver.                             |
| `workspace/action-dir`               | static  | `doctor/checks/workspace.rs::action_dir`                        | #495        | `error` when the configured action directory is not writable.                                   | Create the directory or grant the process write access.                       |
| `secrets/master-key-env`             | static  | `doctor/checks/secrets.rs::master_key_env`                      | #495        | `error` when both master-key environment variables are set.                                     | Keep only the intended master-key source configured.                          |
| `network/proxy`                      | static  | `doctor/checks/network.rs::proxy`                               | #495        | Leaf-defined by the existing proxy validator.                                                   | Correct the proxy configuration reported by the validator.                    |
| `memory/root`                        | static  | `doctor/checks/memory.rs::root`                                 | #495        | Leaf-defined by the existing memory-root validator.                                             | Correct the memory root path reported by the validator.                       |
| `profile/exists`                     | static  | `doctor/profile.rs::exists`                                     | #490        | `error` for a missing profile, unknown layout marker, or invalid profile config.                | Select an existing profile with a supported layout and valid config.          |
| `profile/provisioned`                | connect | `doctor/profile.rs::provisioned`                                | #491        | `error` for an unprovisioned tenant profile.                                                    | Provision the tenant profile before running doctor.                           |
| `workspace/writable`                 | static  | `doctor/checks/workspace.rs::writable`                          | #506        | `error` when the workspace write probe fails.                                                   | Grant write access to the workspace directory.                                |
| `workspace/layout`                   | static  | `doctor/checks/workspace.rs::layout`                            | #506        | Leaf-defined for missing `memory/`, `SYSTEM.md`, or low disk space.                             | Create or repair the reported workspace entry or free disk space.             |
| `runtime/environment`                | static  | `doctor/checks/runtime.rs::environment`                         | #506        | Leaf-defined for git, curl, shell, or home environment checks.                                  | Install or configure the named local prerequisite.                            |
| `runtime/agent-sdk`                  | connect | `doctor/checks/runtime.rs::agent_sdk`                           | #506        | Leaf-defined for the Claude Agent SDK version probe.                                            | Install or configure the agent SDK binary, then rerun doctor.                 |
| `memory/binding`                     | static  | `doctor/checks/memory.rs::binding`                              | #498        | Leaf-defined for the selected memory binding.                                                   | Select an available memory driver.                                            |
| `memory/health`                      | connect | `doctor/checks/memory.rs::health`                               | #498        | `error` when the bound engine is down.                                                          | Restore the memory driver's availability and rerun doctor.                    |
| `storage/connect`                    | connect | `doctor/checks/storage.rs::connect`                             | #498        | `error` when the read-only open fails; never migrate.                                           | Make the configured storage reachable and retry the check.                    |
| `storage/layout`                     | connect | `doctor/checks/storage.rs::layout`                              | #498        | `warn` when a storage layout migration is pending.                                              | Apply the documented migration using the normal storage upgrade path.         |
| `secrets/keyring`                    | connect | `doctor/checks/secrets.rs::keyring`                             | #498        | `skipped` for a non-interactive macOS keychain probe.                                           | Run interactively or configure an available keyring backend.                  |
| `embeddings/configured-model`        | connect | `doctor/checks/embeddings.rs::configured_model`                 | #509        | Non-Ollama providers are `skipped`; probe the configured Ollama URL.                            | Start the configured service or correct its base URL and model.               |
| `modules/<id>`                       | connect | `doctor/checks/modules.rs::module_status`                       | #499        | Failed/Unsupported=`error`; Disabled=`skipped`; Available/Ready=`pass`.                         | Repair or enable the named module source and verify its digest pin.           |
| `mcp/<server>`                       | connect | `doctor/checks/mcp.rs::server_status`                           | #499        | `error` when a configured server fails to connect; `skipped` when `components.mcp` is disabled. | Start the server or correct its connection settings.                          |
| `skills/discovery`                   | static  | `doctor/checks/skills.rs::discovery`                            | #499        | `warn` for a malformed skill manifest.                                                          | Repair the manifest at the reported path.                                     |
| `skills/github-preflight`            | connect | `doctor/checks/skills.rs::github_preflight`                     | #499        | Runs only when a skill requires it; failure severity is leaf-defined.                           | Resolve the reported GitHub access or repository issue.                       |
| `sandbox/backend`                    | connect | `doctor/checks/sandbox.rs::backend`                             | #499        | `error` when the configured Docker backend is unavailable.                                      | Install or start Docker, or select an available sandbox backend.              |
| `channels/<provider>`                | connect | `doctor/checks/channels.rs::provider_health`                    | #500        | `error` on provider health timeout; `skipped` when its component is disabled.                   | Restore the channel provider connection and retry.                            |
| `integrations/composio`              | connect | `doctor/checks/integrations.rs::composio`                       | #500        | `skipped` when its component is disabled; otherwise leaf-defined.                               | Reconnect Composio or correct its endpoint configuration.                     |
| `voice/stt`                          | connect | `doctor/checks/voice.rs::stt`                                   | #500        | `skipped` when `components.voice` is disabled; otherwise leaf-defined.                          | Enable voice or configure an available speech-to-text provider.               |
| `voice/tts`                          | connect | `doctor/checks/voice.rs::tts`                                   | #500        | `skipped` when `components.voice` is disabled; otherwise leaf-defined.                          | Enable voice or configure an available text-to-speech provider.               |
| `search/<provider>`                  | connect | `doctor/checks/search.rs::provider_status`                      | #500        | `warn` when provider status is `NeedsKey`; `skipped` when its component is disabled.            | Configure the provider key named in the finding.                              |
| `wallet/status`                      | connect | `doctor/checks/wallet.rs::status`                               | #500        | `skipped` when its component is disabled; otherwise leaf-defined.                               | Configure the wallet safely or disable the unused wallet feature.             |
| `backend/reachable`                  | connect | `openhuman-tinyhumans/src/doctor/backend.rs::reachable`         | #496        | `error` when backend is down; dependent hosted checks are `skipped`.                            | Check the configured backend URL and network access.                          |
| `backend/credential`                 | connect | `openhuman-tinyhumans/src/doctor/backend.rs::credential`        | #496        | `error` for an expired JWT (`SESSION_EXPIRED`).                                                 | Sign in again through the supported auth flow.                                |
| `backend/identity`                   | connect | `openhuman-tinyhumans/src/doctor/backend.rs::identity`          | #496        | Healthy result passes; failure severity is leaf-defined.                                        | Check tenant identity and access through the supported auth flow.             |
| `backend/socket`                     | connect | `openhuman-tinyhumans/src/doctor/backend.rs::socket`            | #496        | Healthy result passes; failure severity is leaf-defined.                                        | Check backend socket availability and network policy.                         |
| `backend/api-url`                    | static  | `openhuman-tinyhumans/src/doctor/backend.rs::api_url`           | #496        | Healthy result passes; failure severity is leaf-defined.                                        | Set a valid backend API URL for this profile.                                 |
| `inference/managed-catalog`          | connect | `openhuman-tinyhumans/src/doctor/inference.rs::managed_catalog` | #496        | Healthy result passes; failure severity is leaf-defined.                                        | Check the managed inference catalog and tenant access.                        |
| `inference/model-in-catalog`         | connect | `doctor/checks/inference.rs::model_in_catalog`                  | #497        | `warn` when a configured model is absent; `skipped` if provider has no `/models` route.         | Select a listed model or verify provider catalog support.                     |
| `inference/auth-errors`              | connect | `doctor/checks/inference.rs::auth_errors`                       | #497        | `error` per provider in the auth error snapshot.                                                | Resolve provider authentication or access.                                    |
| `inference/live/<role>`              | live    | `doctor/checks/inference_live.rs::role_completion`              | #501        | `error` for a provider 401; other failures are categorized as auth/quota/unavailable.           | Check the role's provider access and model configuration.                     |
| `embeddings/live`                    | live    | `doctor/checks/embeddings_live.rs::embed`                       | #502        | `error` on embed failure or configured/actual dimension mismatch.                               | Check embedding access and configured vector dimensions.                      |
| `memory/roundtrip`                   | live    | `doctor/checks/memory_live.rs::roundtrip`                       | #502        | `error` on roundtrip failure, including insufficient credits.                                   | Check memory-driver write/fetch/delete access and billing.                    |

The IDs above are normative names from the leaves' Scope and Acceptance
criteria. Dynamic segments use the canonical identifier supplied by their
owner; they are safe single path segments, never arbitrary display text.
`config/temperature` appears only in #488's selector test and is a `FakeCheck`
fixture id, not a built-in inventory entry. The former daemon-state check has
no v2 id: #325 deletes it because its state file is never written.

For #491 tenant checks, `profile/provisioned` covers provision and lease
readiness, while `backend/credential`, `inference/provider-ref`, and
`storage/connect` report credential, provider-settings, and storage-scope
readiness within the forced tenant context. A tenant caller cannot use
`profile` to inspect another tenant; an authorized operator may select one.
Authorization is checked before doctor reads config or credentials.

`memory/roundtrip` uses doctor-scoped disposable content and must always forget
it, including after a fetch failure. If cleanup fails, the report names the
cleanup failure without including test content. Static checks may use bounded
temporary local write probes, but must not persist configuration or repair
application state. Connect checks may add read-only network probes but do not
make billable calls or write remote state.

## Implementation references

- Existing doctor being replaced: `crates/openhuman-core/src/doctor/`
- Existing subsystem status projection: `crates/openhuman-core/src/core/subsystem/status.rs`
- Capability ids: `crates/openhuman-core/src/platform/about_app/`
- OpenClaw reference: [structured health-check contract](https://github.com/openclaw/openclaw/blob/main/docs/cli/doctor/health-contract.md), [lint mode and selectors](https://github.com/openclaw/openclaw/blob/main/docs/cli/doctor/lint.md), and [running doctor](https://github.com/openclaw/openclaw/blob/main/docs/cli/doctor/running.md)
