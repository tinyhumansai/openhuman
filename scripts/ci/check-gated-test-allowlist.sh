#!/usr/bin/env bash
# Guard: the set of core source files that #[cfg]-gate a test on a domain
# feature must equal the allowlist below.
#
# Self-maintaining coverage for the feature-gate smoke lane. When a new gate
# (or a new test in an existing gate's domain) adds a gated test file, this
# fails. The author then adds it here AND, if it can hold an ungated-assert
# regression of the #5022 class, extends the gate-contract `cargo test`
# filters run with the gates off (ci-lite.yml `rust-feature-gate-smoke` and
# scripts/ci/self-hosted/lanes-plan.mjs `rust-gates-off`). This keeps the
# smoke lane from silently under-covering as the gate surface grows.
#
# Called by ci-lite.yml and by the lane runner, so the list lives once.
set -euo pipefail

cd "$(dirname "$0")/../.."

EXPECTED=$(cat <<'EOF'
agent_registry/host_definition_tests.rs
agent/session_host/mod.rs
integrations/prelude_integrations.rs
agent/session_host/runtime_session_tool_surface.rs
agent/session_host/runtime_session_tests.rs
agent_registry/agents/loader.rs
agent_registry/agents/loader_tests_specialist_agents_tests.rs
agent/tinyagents/mod.rs
commands/ops.rs
config/migrations/retire_local_whisper_stt_tests.rs
config/schema/load/source/revision_loader_tests.rs
cron/bus_host.rs
cron/bus_host_tests.rs
core/all.rs
core/all_bound_controller_tests.rs
core/all_schema_contract_tests.rs
core/all_tests.rs
core/cli_tests.rs
core/dispatch_tests.rs
core/invoke_tests.rs
core/legacy_aliases_tests.rs
core/runtime/services.rs
core/schema_test_support_tests.rs
core/runtime/subscribers.rs
mcp/server/resources.rs
mcp/ui/tools.rs
mcp/server/tools/mod.rs
modules/mod.rs
modules/ops_components_tests.rs
modules/registry_tests.rs
modules/registry_release_manifest_tests.rs
modules/runtime.rs
modules/runtime_tests.rs
platform/socket/event_handlers.rs
profiles/surface_tests.rs
threads/store/mod.rs
agent/session_host/transcript_compat_tests.rs
channels/mod.rs
mcp/host.rs
modules/registry_contract_tests.rs
modules/registry.rs
credentials/ops/credential.rs
storage/import/tinyflows.rs
storage/schema.rs
skills/bundled/mod.rs
skills/mod.rs
skills/search.rs
tools/impl/network/host.rs
tools/impl/network/host_tests.rs
tools/ops.rs
tools/ops_tests.rs
tools/ops_tests_capability_gating_tests.rs
tools/ops_tests_default_registry_tests.rs
tools/ops_tests_domain_family_tests.rs
tools/registry/ops_tests.rs
tools/registry/schemas_tests.rs
tools/sources_tests.rs
tools/ops_tests_catalog_fixture_tests.rs
web_chat/mod.rs
web_chat/web_tests.rs
web_chat/web_tests_ptt_source_tests.rs
web3/stub.rs
web3/wallet/stub.rs
web3/x402/stub.rs
profiles/surface_tests.rs
threads/store/mod.rs
agent/session_host/transcript_compat_tests.rs
channels/mod.rs
mcp/host.rs
modules/registry_contract_tests.rs
modules/registry.rs
memory/ops.rs
memory/ops_tests.rs
agent/prompts/mod.rs
agent/session_host/runtime_session_flow_prompt_tests.rs
agent/subagent_host/ops/runner_flow_prompt_tests.rs
agent_registry/agents/fleet_prompt_tests.rs
agent_registry/agents/mod.rs
core/agent_cli/debug/mod.rs
core/agent_cli/debug/mod_tests.rs
core/runtime/services_tests.rs
integrations/composio/ops/mod.rs
integrations/composio/client.rs
mcp/server/resources_tests.rs
modules/tinyflows.rs
doctor/checks/channels.rs
doctor/checks/inference_tests.rs
doctor/checks/mcp.rs
doctor/checks/skills.rs
doctor/checks/voice.rs
doctor/checks/wallet.rs
doctor/checks/wallet_tests.rs
skills/ops_discover.rs
agent/host_agents.rs
agent/turn_host.rs
cron/mod.rs
cron/scheduler/delivery.rs
cron/scheduler/origin_delivery.rs
agent/queued_turn_tests.rs
cron/ops_tests.rs
cron/scheduler_tests.rs
web_chat/web_tests_turn_lifecycle_tests.rs
channels/host/voice_tests.rs
EOF
)

ACTUAL=$(node scripts/ci/list-feature-gated-rust-tests.mjs | sort -u)
if ! diff <(printf '%s\n' "$EXPECTED" | sort -u) <(printf '%s\n' "$ACTUAL"); then
  echo "::error::Gated-test file set changed. Update EXPECTED in scripts/ci/check-gated-test-allowlist.sh, and extend the gate-contract 'cargo test' filters if the new module can carry an ungated-assert regression (see #5022)."
  exit 1
fi
echo "gate-contract test coverage allowlist is current"
