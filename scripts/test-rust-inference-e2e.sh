#!/usr/bin/env bash
# Run the inference provider E2E tests (tests/inference_provider_e2e.rs).
#
# These tests use wiremock to mock HTTP upstreams — no live LLM API is needed.
# They exercise:
#   - OpenAI-compatible chat and streaming paths
#   - Anthropic auth style header verification
#   - Per-model temperature suppression (o1/o3/o4/gpt-5 patterns)
#   - Ollama local provider (via OpenAI-compat /v1 endpoint)
#   - /v1/chat/completions and /v1/models HTTP endpoint auth layer
#
# Usage:
#   bash scripts/test-rust-inference-e2e.sh
#
# Via Docker (Linux):
#   docker compose -f e2e/docker-compose.yml run --rm inference-e2e
#
# The shared mock backend is NOT required by these tests (they use wiremock
# directly), but this script delegates to test-rust-with-mock.sh for
# consistency with the rest of the Rust test runner tooling.
set -euo pipefail
exec bash "$(dirname "$0")/test-rust-with-mock.sh" --test inference_provider_e2e "$@"
