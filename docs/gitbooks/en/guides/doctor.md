---
description: >-
  Run OpenHuman diagnostics, choose a check tier, and use the report to fix
  configuration and runtime problems.
icon: activity
---

# Diagnose OpenHuman with doctor

The doctor command checks the selected OpenHuman configuration and runtime. It
returns stable check ids, a result for each check, and a repair hint. It does
not repair settings or change the active profile.

## Run a report

Open a terminal where `openhuman-core` is installed and run:

```bash
openhuman-core doctor
```

The default `connect` tier checks local configuration and workspace state, then
runs bounded, read-only checks for configured services. Use `--live` only to
opt into live checks:

```bash
openhuman-core doctor --live
```

The CLI uses `connect` by default. Its `--live` option selects live checks;
the `static` tier is available through JSON-RPC and the agent tool.

Use `--only` and `--skip` to select exact check ids. Repeat either option to
select more than one check:

```bash
openhuman-core doctor --only config/parse --only workspace/writable
openhuman-core doctor --skip embeddings/configured-model
```

An unknown id appears as a `doctor/selection` error in the report. Valid ids
in the same request still run. The full inventory and fix hints are below.

## Tiers and cost

| Tier      | What it runs                                                                                                               |
| --------- | -------------------------------------------------------------------------------------------------------------------------- |
| `static`  | Local config and filesystem checks. Some checks briefly create and remove a bounded temporary file to verify access.       |
| `connect` | Static checks plus bounded, read-only checks against configured local services and network endpoints. This is the default. |
| `live`    | Connect checks plus one-shot inference and embedding requests and a disposable memory store/fetch/forget roundtrip.        |

{% hint style="warning" %}
Live checks may spend inference or memory-service credits. Run `--live` only
when you intend to make those requests. The memory roundtrip stores an isolated
temporary item and attempts to remove it afterward; the report says when it
cannot confirm cleanup. A local SQLite memory engine may update derived
`-shm` or empty `-wal` sidecar state during database access.
{% endhint %}

Each chat inference probe sends the short `Reply OK` prompt with a provider-
enforced output limit of at most five tokens. Roles that resolve to the same
endpoint, credential, and wire model share one completion while keeping separate
findings. A route that cannot enforce an output limit in its request is skipped
without a model call. OpenAI Codex OAuth uses a Responses API request that has
no output-token limit field, so its `inference/live/<role>` check is skipped;
that skip does not indicate that Codex is healthy.

The agent tool follows the same rule: ask the assistant to run `doctor_health`
for ordinary diagnostics. It should request the live tier only when you
explicitly ask for billable probes.

## Output modes and exit codes

The command supports a human-readable table, JSON output, and lint mode:

```bash
openhuman-core doctor --json
openhuman-core doctor --lint
openhuman-core doctor --lint --severity-min warn --json
```

| Mode            | Result                                                                                                                       |
| --------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| Default         | Prints a table grouped by check category and a summary. Returns `0` after a report is produced.                              |
| `--json`        | Prints the v2 report as JSON and returns `0` after a report is produced, regardless of findings.                             |
| `--lint`        | Returns `0` when no finding meets the minimum severity, `1` when one or more do, or `2` if the report could not be produced. |
| `--lint --json` | Prints the report as JSON and keeps lint's `0`, `1`, or `2` exit behavior.                                                   |

If no report can be produced, the command exits `2` with a short reason, for
example `doctor could not produce a report: the doctor host could not start
(profile selection, config or core boot)`. With `--json` it prints
`{"ok": false, "reason": "…"}` instead. `openhuman-core doctor --help` lists
every option.

`--severity-min` accepts `info`, `warn`, or `error` and defaults to `error`.
Pass and skipped findings never meet the threshold. For example, to fail CI on
warnings as well as errors:

```bash
openhuman-core doctor --lint --severity-min warn
```

## Select a profile

On desktop, `--profile <id>` checks a profile without activating it or
switching the current user:

```bash
openhuman-core doctor --profile work
```

Profile loading is read-only. A missing profile or an unsupported layout is
reported with a finding and a fix hint. In a SaaS host, diagnostics use the
current tenant's forced configuration; a profile option cannot select another
tenant.

## Read the report

The report uses schema version `2`. It includes the selected profile and config
source when known, requested tier, check ids run or skipped, findings, a
severity summary, and `ok`. `ok` is false when a finding has severity `error`;
warnings remain advisory. In lint mode, the separate severity threshold
determines the exit code.

Each finding has a short message and a `fix_hint`. The report may name a config
path and whether a value came from the command line, environment, `.env`, file,
or default. It does not include credentials or resolved config values. The
`config/source` finding may identify the loaded config and `.env` file paths so
you can diagnose where a setting came from.

The doctor does not have a `--fix` mode. Apply a repair yourself, then rerun
the reported check or the full report.

## Check inventory

The ids below are registered checks. Dynamic ids include the configured module,
server, provider, or inference role in the final path segment. Each finding
includes a more specific hint for the setting or service it checked.

| Check id                             | Tier    | What it checks                                                                                                                                                                                                  | Next step when it reports a problem                                                                                                                                                         |
| ------------------------------------ | ------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `config/parse`                       | static  | Config parsing and recovery from an invalid file or backup.                                                                                                                                                     | Repair `config.toml`; inspect and restore a valid backup if recovery occurred.                                                                                                              |
| `config/unknown-key`                 | static  | Config keys ignored because they are not supported.                                                                                                                                                             | Rename the key to the suggested supported field.                                                                                                                                            |
| `config/legacy-key`                  | static  | Legacy config keys migrated during load.                                                                                                                                                                        | Update the config to the current key name.                                                                                                                                                  |
| `config/env-override`                | static  | Values supplied by environment variables or `.env` that shadow file values.                                                                                                                                     | Choose one source and remove the shadowed override.                                                                                                                                         |
| `config/source`                      | static  | Config and `.env` paths and value-source information.                                                                                                                                                           | Use the reported source to diagnose precedence.                                                                                                                                             |
| `inference/provider-ref`             | static  | References to unavailable inference providers.                                                                                                                                                                  | Select a registered provider and correct the reported config field.                                                                                                                         |
| `inference/construct`                | static  | Whether each configured model route can be constructed with its required credential.                                                                                                                            | Correct provider settings and configure its required credential.                                                                                                                            |
| `inference/temperature`              | static  | Supported temperature settings on configured model routes.                                                                                                                                                      | Set a supported temperature for the reported route.                                                                                                                                         |
| `embeddings/config`                  | static  | Embedding provider, model, route, and vector dimensions.                                                                                                                                                        | Configure a provider, model, and dimension that agree.                                                                                                                                      |
| `components/referenced-but-disabled` | static  | Config that refers to a disabled component.                                                                                                                                                                     | Enable the component or remove the dependent config.                                                                                                                                        |
| `config/runtime-settings`            | static  | Ignored fallback providers and incomplete model, delegate, or enabled-channel settings.                                                                                                                         | Use the path-specific hint to complete, remove, or enable the named setting.                                                                                                                |
| `modules/source-fallback`            | static  | Module source selection that fell back from the intended source.                                                                                                                                                | Configure the intended module source explicitly.                                                                                                                                            |
| `modules/override-invalid`           | static  | Invalid module overrides.                                                                                                                                                                                       | Correct or remove the reported override.                                                                                                                                                    |
| `storage/url`                        | static  | Whether the configured storage URL uses a driver available in this build.                                                                                                                                       | Use a supported URL scheme and include its driver.                                                                                                                                          |
| `workspace/action-dir`               | static  | Whether the agent action directory is usable.                                                                                                                                                                   | Create the directory or grant the process access.                                                                                                                                           |
| `workspace/writable`                 | static  | Whether the selected workspace permits a bounded write probe.                                                                                                                                                   | Grant write access to the workspace directory.                                                                                                                                              |
| `workspace/layout`                   | static  | Required workspace entries and available disk space.                                                                                                                                                            | Create the named entry or free disk space.                                                                                                                                                  |
| `runtime/environment`                | static  | Required local environment, including `HOME`, `SHELL`, `git`, and `curl`.                                                                                                                                       | Install or configure the named local prerequisite.                                                                                                                                          |
| `runtime/agent-sdk`                  | connect | The configured Claude Agent SDK command and version.                                                                                                                                                            | Install or configure the SDK command, then rerun doctor.                                                                                                                                    |
| `secrets/master-key-env`             | static  | Whether both master-key environment variables are set.                                                                                                                                                          | Keep only the intended master-key source configured.                                                                                                                                        |
| `network/proxy`                      | static  | Whether the configured network proxy is valid.                                                                                                                                                                  | Correct the proxy setting reported by the check.                                                                                                                                            |
| `memory/root`                        | static  | Whether the configured memory root is valid.                                                                                                                                                                    | Correct the reported memory-root path.                                                                                                                                                      |
| `profile/exists`                     | static  | Whether a selected desktop profile exists and has a supported layout and valid config.                                                                                                                          | Select an existing profile with a supported layout and valid config.                                                                                                                        |
| `profile/provisioned`                | connect | Whether the selected tenant profile is provisioned and ready.                                                                                                                                                   | Provision the tenant profile before running doctor again.                                                                                                                                   |
| `memory/binding`                     | static  | The selected memory engine and its configuration.                                                                                                                                                               | Select an available memory driver.                                                                                                                                                          |
| `memory/health`                      | connect | Whether the selected memory engine is healthy.                                                                                                                                                                  | Restore the memory driver's availability and rerun the check.                                                                                                                               |
| `memory/roundtrip`                   | live    | Whether a disposable memory item can be stored, fetched, and removed.                                                                                                                                           | Check the selected engine; inspect the isolated doctor scope if cleanup was not confirmed.                                                                                                  |
| `storage/connect`                    | connect | Read-only access to the configured storage driver. A missing SQLite database is not created; an existing database and WAL data remain intact, though SQLite may create derived `-shm` or empty `-wal` sidecars. | Check that the configured storage exists and is readable; connect to remote storage if it is unavailable.                                                                                   |
| `storage/layout`                     | static  | An interrupted profile storage-layout migration.                                                                                                                                                                | Stop OpenHuman and resume the migration through Settings or the CLI.                                                                                                                        |
| `secrets/keyring`                    | connect | Whether the configured keyring can be inspected safely. Non-interactive macOS checks may be skipped.                                                                                                            | Run interactively or configure an available keyring backend.                                                                                                                                |
| `embeddings/configured-model`        | connect | Whether the configured Ollama embedding model is available. Other providers are skipped by this check.                                                                                                          | Start the configured Ollama service or correct its URL and model.                                                                                                                           |
| `modules/<id>`                       | connect | Status of each configured module. Disabled modules are skipped.                                                                                                                                                 | Repair or enable the module and verify its configured digest.                                                                                                                               |
| `mcp/<server>`                       | connect | Connectivity for each configured MCP server.                                                                                                                                                                    | Start the server or correct its connection settings.                                                                                                                                        |
| `skills/discovery`                   | static  | Whether installed skill manifests can be read.                                                                                                                                                                  | Repair the manifest at the reported path.                                                                                                                                                   |
| `skills/github-preflight`            | connect | GitHub access required by an installed skill.                                                                                                                                                                   | Resolve the reported GitHub access or repository issue.                                                                                                                                     |
| `sandbox/backend`                    | connect | Whether the configured Docker sandbox backend is available.                                                                                                                                                     | Install or start Docker, or choose an available sandbox backend.                                                                                                                            |
| `channels/<provider>`                | connect | Health of each configured messaging provider.                                                                                                                                                                   | Restore the provider connection and rerun the check.                                                                                                                                        |
| `integrations/composio`              | connect | Composio availability for the selected profile.                                                                                                                                                                 | Reconnect Composio or correct its endpoint configuration.                                                                                                                                   |
| `voice/stt`                          | connect | Readiness of the configured speech-to-text provider.                                                                                                                                                            | Enable voice or configure a supported speech-to-text provider.                                                                                                                              |
| `voice/tts`                          | connect | Readiness of the configured text-to-speech provider.                                                                                                                                                            | Enable voice or configure a supported text-to-speech provider.                                                                                                                              |
| `search/<provider>`                  | connect | Readiness of each configured search provider.                                                                                                                                                                   | Configure the provider key named in the finding.                                                                                                                                            |
| `wallet/status`                      | connect | Readiness of the configured wallet component.                                                                                                                                                                   | Configure the wallet safely or disable the unused wallet feature.                                                                                                                           |
| `backend/api-url`                    | static  | Whether the hosted backend URL is valid for the selected profile.                                                                                                                                               | Set a valid backend API URL for this profile.                                                                                                                                               |
| `backend/reachable`                  | connect | Whether the hosted backend responds to its health check.                                                                                                                                                        | Check the configured backend URL and network access.                                                                                                                                        |
| `backend/credential`                 | connect | Whether the selected profile has a usable hosted credential.                                                                                                                                                    | Sign in again through the supported auth flow.                                                                                                                                              |
| `backend/identity`                   | connect | Whether the backend accepts the selected identity.                                                                                                                                                              | Check the tenant identity and access through the supported auth flow.                                                                                                                       |
| `backend/socket`                     | connect | Current backend socket connection state.                                                                                                                                                                        | Check socket availability and the profile's network policy.                                                                                                                                 |
| `inference/managed-catalog`          | connect | Whether the hosted managed inference catalog is reachable.                                                                                                                                                      | Check managed inference access for this profile.                                                                                                                                            |
| `inference/model-in-catalog`         | connect | Whether configured models appear in their provider catalogs. Providers without a read-only catalog are skipped.                                                                                                 | Choose a listed model or verify catalog support for the provider.                                                                                                                           |
| `inference/auth-errors`              | connect | Authentication errors recorded for inference providers.                                                                                                                                                         | Update or reconnect the provider credentials.                                                                                                                                               |
| `inference/live/<role>`              | live    | Whether each configured inference role can return a short, capped completion. Routes that cannot enforce the output cap are skipped; identical routes share one request.                                        | A skipped check made no model call and does not confirm provider health. For attempted requests, check the provider, model, credential, quota, and network connection named by the finding. |
| `embeddings/live`                    | live    | Whether the selected embedding provider returns the configured vector width.                                                                                                                                    | Check provider access and align the configured model and dimensions.                                                                                                                        |

## Common interpretation

- A `pass` means the check completed successfully. A `warn` calls out a problem
  that does not make the report's `ok` field false. An `error` means a check
  found a failure. A `skipped` result explains why a check did not run.
- A check above the selected tier, behind a disabled component, or using an
  inference route that cannot enforce the output cap is skipped; a skipped check
  is not evidence that the service is healthy.
- `doctor.models` is a separate read-only provider-catalog report. It distinguishes
  available catalogs, providers without a catalog, access failures, and request
  errors; it does not make inference requests.
