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
agent/harness/builtin_definitions_tests.rs
agent/harness/definition_tests.rs
agent/session_host/builder/factory.rs
agent/session_host/runtime_session.rs
agent/session_host/runtime_session_tests.rs
agent/session_host/turn/tools.rs
agent/subagent_host/tool_prep_tests.rs
agent/registry/agents/loader.rs
agent/registry/agents/loader_tests_builtin_registration_tests.rs
agent/registry/agents/loader_tests_orchestrator_tier_tests.rs
agent/registry/agents/loader_tests_specialist_agents_tests.rs
agent/registry/agents/mod.rs
agent/tinyagents/mod.rs
commands/ops.rs
config/migrations/retire_local_whisper_stt_tests.rs
core/all.rs
core/all_tests.rs
core/cli_tests.rs
core/dispatch_tests.rs
core/jsonrpc.rs
core/jsonrpc_tests.rs
core/legacy_aliases_tests.rs
core/runtime/services.rs
flows/mod.rs
mcp/server/resources.rs
mcp/server/tools/mod.rs
platform/socket/event_handlers.rs
skills/bundled/mod.rs
skills/mod.rs
skills/search.rs
tools/impl/network/http_request.rs
tools/ops.rs
tools/ops_tests.rs
tools/ops_tests_capability_gating_tests.rs
tools/ops_tests_default_registry_tests.rs
tools/ops_tests_domain_family_tests.rs
tools/registry/ops_tests.rs
tools/registry/schemas_tests.rs
voice/compile_status_tests.rs
web3/mod.rs
web3/stub.rs
web3/wallet/stub.rs
web3/x402/stub.rs
EOF
)

ACTUAL=$(node scripts/ci/list-feature-gated-rust-tests.mjs | sort -u)
if ! diff <(printf '%s\n' "$EXPECTED" | sort -u) <(printf '%s\n' "$ACTUAL"); then
  echo "::error::Gated-test file set changed. Update EXPECTED in scripts/ci/check-gated-test-allowlist.sh, and extend the gate-contract 'cargo test' filters if the new module can carry an ungated-assert regression (see #5022)."
  exit 1
fi
echo "gate-contract test coverage allowlist is current"
