# Polymarket Integration (Read + Trading)

This document describes the Polymarket integration for issue #1398.

## Scope

The `polymarket` tool now supports both market browsing and trading workflows over:

- Gamma API (`https://gamma-api.polymarket.com`)
- CLOB API (`https://clob.polymarket.com`)

Supported read actions:

- `list_markets`
- `get_market`
- `list_events`
- `get_orderbook`
- `get_price`
- `get_positions`
- `get_balance`
- `get_open_orders`
- `get_usdc_allowance`

Supported write actions:

- `place_order`
- `cancel_order`

## Architecture

Implementation lives in `src/openhuman/tools/impl/network/polymarket.rs` with helper modules:

- `clob_auth.rs`: L1 credential derivation + L2 HMAC headers
- `polymarket_orders.rs`: EIP-712 order typed-data signing

Key runtime behavior:

- Layer-2 API credentials are derived on first authenticated call and cached.
- Derived credentials are persisted to `integrations.polymarket.derived_clob_credentials` (plain config fallback until secret-store migration lands).
- Order placement fetches `GET /nonce?user=<eoa>` before signing to avoid replay/nonce mismatch.
- USDC.e allowance is read via Polygon `eth_call` against ERC-20 `allowance(owner, spender)`.

## Authentication and Signing Flow

### L1 handshake (one-time bootstrap)

- Sign CLOB `ClobAuth` EIP-712 payload with Polygon chain id `137`.
- Call `POST /auth/api-key`; if needed, fall back to `GET /auth/derive-api-key`.
- Persist returned `{ apiKey, secret, passphrase }` for L2 usage.

### L2 authenticated requests

Each authenticated CLOB request signs:

- `timestamp + method + request_path (+ body for POST)`

Headers:

- `POLY_ADDRESS`
- `POLY_SIGNATURE`
- `POLY_TIMESTAMP`
- `POLY_NONCE: 0`
- `POLY_API_KEY`
- `POLY_PASSPHRASE`

### Order signing

`place_order` signs an EIP-712 order using domain:

- name: `Polymarket CTF Exchange`
- version: `1`
- chain id: `137`
- verifying contract: `integrations.polymarket.clob_exchange_contract`

## Permissions

Write actions are currently guarded by an explicit stopgap approval flag.

- `place_order` and `cancel_order` require `approved=true`.
- If omitted or `false`, the tool returns:
  - `Polymarket write requires explicit user approval. Re-invoke with arguments.approved = true after confirming with the user.`

This is temporary until the shared approval gate from #1339 is integrated.

## Configuration

Config path: `integrations.polymarket`.

Fields:

- `enabled` (default `false`)
- `gamma_base_url` (default `https://gamma-api.polymarket.com`)
- `clob_base_url` (default `https://clob.polymarket.com`)
- `timeout_secs` (default `15`)
- `eoa_address` (optional default user address)
- `polygon_rpc_url` (default `https://polygon-rpc.com`)
- `usdc_contract` (default `0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174`)
- `clob_exchange_contract` (default `0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E`)
- `derived_clob_credentials` (optional cached L2 credentials)

## USDC Allowance Contract

`get_usdc_allowance` reports approval state only; it does not mutate chain state.

- Token: USDC.e on Polygon (`0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174`)
- Spender: Polymarket exchange (`0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E`)

If allowance is insufficient, approval must be executed separately (wallet tool / explicit user-approved flow).

## Error and Retry Behavior

- 4xx errors are treated as client errors and are not retried.
- 429 and 5xx errors are treated as transient and retried up to 3 attempts.
- Backoff is fixed at 500ms between retries.
- Timeouts surface as explicit deadline errors.

## Test Strategy

Unit tests are in `src/openhuman/tools/impl/network/polymarket_tests.rs` plus helper-module tests.

- Existing read-path and retry behavior tests remain covered.
- Added coverage for authenticated read actions, write approval gating, and Polygon allowance reads.
- `clob_auth.rs` tests cover HMAC/header fixture behavior.
- `polymarket_orders.rs` tests cover domain and deterministic signing fixture behavior.
