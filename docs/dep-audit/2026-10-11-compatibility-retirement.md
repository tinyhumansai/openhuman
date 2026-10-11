# Compatibility retirement decisions (2026-10-11)

Tracking: [OpenHuman #7409](https://github.com/tinyhumansai/openhuman/issues/7409).
The contributor build is v0.64.15. Retained OpenHuman entries are reviewed after
2027-01-11 for removal in v0.66; that is a review boundary, not an automatic
promise to discard old user data. Removal requires the evidence stated below.

| Item                                      | Decision and retirement evidence                                                                                                                                                                                                                                                                         |
| ----------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Server and frontend legacy RPC tables     | Keep together until v0.66. Successful calls grouped by raw incoming method and shipped client version must establish that old names are unused. Error-only telemetry cannot establish that.                                                                                                              |
| CloudProviderType and provider type       | Remove the enum and persisted typed discriminator. Incoming RPC type stays until v0.66 pending shipped-client parameter coverage. Decode old TOML through the schema-14 migration before typed deserialization; persist the normalized config once. Explicit endpoint/auth/slug/label values win.        |
| Provider default_model                    | Keep until v0.66: this is an active BYOK model fallback, including abstract-model remapping in factory/cloud_slug.rs, and an accepted inference settings input. Persist it instead of dropping it on every save. Removing it requires a replacement model contract and migration of all workload routes. |
| Deprecated local-AI usage helpers         | Remove; update callers to Config::workload_uses_local, whose provider route is authoritative. Legacy usage booleans remain input to older config migrations.                                                                                                                                             |
| Tool-policy request mirrors               | Remove session_id, channel, and agent_definition_id copies; consumers use request.context. Retain the existing sensitive-field debug redaction.                                                                                                                                                          |
| Skill metadata discovery entry            | Keep until v0.66. It now caches results and applies ambient user-root visibility, trust and tenant scope; replacing it with raw discovery would change behavior. Existing scope/cache tests cover it.                                                                                                    |
| Integration-cache expiry alias            | Remove; callers use cached_active_integrations, which already preserves the same last-known credential-scoped snapshot without a time expiry.                                                                                                                                                            |
| Credential-gated service historical names | Remove; migrate all direct callers to start_credential_gated_services / stop_credential_gated_services.                                                                                                                                                                                                  |
| skill_manage tool-family alias            | Keep until v0.66. Persisted opt-in snapshots require a one-shot skill_manage -> workflow_manage migration before this default-off family can disappear.                                                                                                                                                  |
| Subagent archetype parameter              | Keep until v0.66. Current prompts use agent_id, but resumed sessions freeze tool schemas and existing native tests pin old argument acceptance. Local source searches cannot establish the contents of every user's durable transcript.                                                                  |
| Unscoped cancel wrapper                   | Remove the wrapper; current callers explicitly pass None to cancel_chat_scoped, preserving whole-thread cancellation and existing stale-request tests.                                                                                                                                                   |
| Legacy inference_url route fallback       | Keep until v0.66. The URL remains accepted by config.update and headless settings, and active completion logic pins BYOK roles. Dropping the fallback without completing upgrades can change which account pays for inference.                                                                           |
| voice_cloud_transcribe                    | Keep until v0.66. app/src/features/human/voice/sttClient.ts still sends it, with frontend and JSON-RPC regressions. Migrate that shipped caller before evaluating removal.                                                                                                                               |
| Legacy Ready artifact migration           | Keep until v0.66, pending an explicit pre-files upgrade/import boundary for offline workspaces. Crash-safe copy/metadata handling and idempotence remain intact.                                                                                                                                         |
| Legacy timestamped session adoption       | Owned by TinyAgents, not core. [TinyAgents #390](https://github.com/tinyhumansai/tinyagents/pull/390) dates review to 2027-01-11 and removal to v3.0, requiring an explicit host upgrade/import boundary. No unpublished gitlink is consumed here.                                                       |

## Production evidence limits

Read-only Sentry queries ran on 2026-10-11 against organization tinyhumans.
The project map is not configured for this checkout; the known tauri-rust
project accepts queries. Its issue endpoint rejects a 90d stats period and
accepts 14d. An exact "openhuman.mcp_list" query returned zero matching issues.
That says nothing about successful calls, and the issue endpoint's event count
is a lifetime issue total rather than a shipped-client method count.

The initial Discover query for rpc.method returned no rows. Source inspection
established that the RPC error reporter tags method, domain=rpc and
operation=invoke_method instead. Querying those tags over 14d found one event
for the retained alias tool_registry.diagnostics on shipped release
openhuman@0.57.52+35b7073d6a3a. This is positive evidence against retiring that
alias. These counts cover reported failures rather than every successful call;
they cannot establish that the other old names are unused. No analytics
connector or successful-call method-count helper is available in this session.
Accordingly both RPC alias tables remain with synchronized dated comments; no
user payloads or credentials were copied.

## Verification

The old-provider fixture covers type-to-slug/default endpoint/auth migration
and model survival across save/reload. Migration fixtures also cover explicit
values, unknown provider slugs, malformed discriminator rejection, idempotence,
a schema-14 no-op, and failed-save retry. A real JSON-RPC fixture covers both
settings controllers retaining incoming type defaults while preserving explicit
endpoint/auth values. Existing native tests cover policy-context redaction,
cache fallback, route-based local usage and scoped cancellation.

The issue names scripts/e2e-core.sh, which is absent at this base. The supported
full-product mock-backed runner is scripts/test-rust-e2e.sh --suite json_rpc_e2e.
This change verifies affected JSON-RPC cases directly through the native Cargo
target; that does not claim a full-product runner or compose pass. Actual
command results belong in the PR body.
