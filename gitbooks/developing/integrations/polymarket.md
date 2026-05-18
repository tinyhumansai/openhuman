# Polymarket Integration (v1 Read-Only)

This document describes the Phase 1 Polymarket integration for issue #1398.

## Scope

v1 ships a read-only `polymarket` tool that calls public Polymarket endpoints:

- Gamma API (`https://gamma-api.polymarket.com`)
- CLOB read API (`https://clob.polymarket.com`)

Supported actions:

- `list_markets`
- `get_market`
- `list_events`
- `get_orderbook`
- `get_price`

Non-goals in v1:

- No wallet signing
- No order placement or cancellation
- No account/position mutation

## Architecture

Implementation lives in `src/openhuman/tools/impl/network/polymarket.rs`.

- Tool name: `polymarket`
- Category: `skill`
- Transport: `reqwest` (GET-only)
- Config surface: `integrations.polymarket` in `config.toml`

Config fields:

- `enabled` (default `true`)
- `gamma_base_url` (default `https://gamma-api.polymarket.com`)
- `clob_base_url` (default `https://clob.polymarket.com`)
- `timeout_secs` (default `15`)

## Error and Retry Behavior

- 4xx errors are treated as client errors and are not retried.
- 429 and 5xx errors are treated as transient and retried up to 3 attempts.
- Backoff is fixed at 500ms between retries.
- Timeouts surface as explicit deadline errors.

## Test Strategy

Unit tests are in `src/openhuman/tools/impl/network/polymarket_tests.rs`.

- Uses static fixtures under `tests/fixtures/polymarket/`.
- Uses an embedded `tokio::net::TcpListener` mock server.
- Covers happy paths for all actions plus 4xx, 5xx, timeout, and schema parsing.

## Deferred Phase (v2+)

Planned follow-up for #1398:

- Trading writes through `rs-clob-client-v2`
- Polygon wallet signing integration
- Phase-appropriate agent wiring for trade execution flows

Kalshi and Hyperliquid integrations are tracked as separate follow-up issues.

## Example Agent Prompts

- "List active Polymarket markets about Ethereum with limit 10."
- "Get Polymarket market details for slug `will-eth-hit-10k`."
- "Show the Polymarket orderbook for token `1001`."
- "Get buy-side price on Polymarket for token `1001`."
