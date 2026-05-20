# Kalshi Integration (v1)

`kalshi` adds Kalshi market reads plus authenticated portfolio/trading actions for issue #1398.

## Endpoints

Base URL: `https://api.elections.kalshi.com/trade-api/v2`

Public reads:

- `list_markets`
- `get_market`
- `list_events`
- `get_event`
- `get_orderbook`

Authenticated reads:

- `get_positions`
- `get_balance`
- `get_open_orders`
- `get_fills`

Writes:

- `place_order`
- `cancel_order`

## Auth

Configured in `integrations.kalshi.credentials`:

- `api_key`
- `private_key_pem` (RSA-PSS-SHA256)
- `secret` (HMAC-SHA256)

Headers:

- `KALSHI-ACCESS-KEY`
- `KALSHI-ACCESS-TIMESTAMP` (unix milliseconds)
- `KALSHI-ACCESS-SIGNATURE`

## Safety

- `place_order` and `cancel_order` require `approved=true`.
- `place_order` generates `client_order_id` as UUIDv4.
- `yes_price` / `no_price` are validated as cents (`1..=99`).

## Config

Path: `integrations.kalshi`

- `enabled` (default `true`)
- `base_url`
- `timeout_secs`
- `credentials`
