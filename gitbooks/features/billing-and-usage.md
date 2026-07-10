---
description: >-
  Plans, credits and saved cards over Stripe and Coinbase, plus a local
  real-time dashboard for token usage, cost and budget enforcement.
icon: credit-card
---

# Billing, Cost & Usage

OpenHuman keeps two related but separate ledgers. **Billing** is what you pay the hosted backend: plans, credit top-ups, saved cards and coupons, all settled through Stripe or Coinbase. **Cost & Usage** is what the agent spends on your behalf, tracked locally per provider call so you can see (and cap) real token spend before the bill ever lands.

The first lives in the cloud; the second never leaves your workspace.

***

## Part 1: Billing & Payments

The `billing` domain is a thin RPC adapter. It holds **no payment logic or state of its own**. Every operation forwards an authenticated HTTPS call to the hosted backend (`/payments/*`, `/coupons/*`) using your stored app-session JWT, and surfaces the JSON response verbatim. Authorization, plan ownership and payment policy are all enforced backend-side. A missing or invalid session yields the backend's `401`/`403` directly; JWTs and card data are never logged.

Pre-HTTP, the adapter does light input validation only: non-empty plan/coupon/payment-method ids, a finite positive `amountUsd`, and a gateway whitelist of `stripe` / `coinbase`.

### Plans

Three tiers are offered, each with a monthly and annual interval:

| Tier | Monthly | Annual | Per-call discount vs pay-as-you-go |
| --- | --- | --- | --- |
| **Free** | $0 | $0 | None (pay-as-you-go baseline) |
| **Basic** | $19.99 | $199 | 50% cheaper per call |
| **Pro** | $199.99 | $1,799.99 | 90% cheaper per call |

Higher tiers do not unlock features so much as lower the **per-call margin** over the pay-as-you-go baseline. All tiers have "access to everything"; you are buying cheaper inference, not gated capabilities.

### Payment providers

Two gateways are wired, and only two:

- **Stripe**: plan purchases (Checkout sessions), the customer billing portal, credit top-ups, saved-card management (SetupIntents) and auto-recharge.
- **Coinbase Commerce**: crypto charges, used for credit top-ups and annual billing.

`top_up_credits` and `create_coinbase_charge` default to the `stripe` gateway and `annual` interval; an empty or whitespace gateway normalises to Stripe.

### Credits, top-ups & auto-recharge

Beyond a subscription you hold a **USD credit balance**. You can read the balance, page through transaction history, and top up via either gateway. **Auto-recharge** (Stripe only) re-fills credits from a saved card when the balance runs low; you can read and update its settings, and list / add / update / delete saved cards. Adding a card creates a Stripe SetupIntent; deleting one is treated as a dangerous operation.

### Coupons

Coupon codes are redeemed against the backend (`POST /coupons/redeem`), and you can list the coupons currently redeemed on your account (`GET /coupons/me`).

### Where billing lives in the app

The desktop **Settings → Billing** panel intentionally has no embedded payment UI. It links out to the hosted web **billing dashboard**, which is the single place to manage plans, cards and invoices. The agent can also read billing state through default-ON tools (plan, balance, transactions, cards, coupons, the Stripe portal link); every money-moving or payment-method mutator ships **default-OFF** behind a `billing_writes` toggle, and card deletion is flagged dangerous.

### RPC surface

Namespace `billing`, exposed as `openhuman.billing_*` (15 methods), e.g. `billing_get_current_plan`, `billing_get_balance`, `billing_get_transactions`, `billing_purchase_plan`, `billing_top_up`, `billing_create_coinbase_charge`, `billing_get_cards`, `billing_create_setup_intent`, `billing_update_auto_recharge`, `billing_redeem_coupon`.

***

## Part 2: Cost & Usage Dashboard

The `cost` domain is entirely local. It records every provider call's token usage and computed USD cost to an append-only JSONL file (`<workspace>/state/costs.jsonl`), keeps in-memory daily/monthly aggregates, enforces budgets, and serves a 7-day dashboard over JSON-RPC. A process-global singleton tracker is shared by the agent turn loop (which logs telemetry after each provider call) and the dashboard handlers, so each call is persisted exactly once.

### Real-time token & cost tracking

For each call, per-call cost is computed from token counts and per-million-token prices (clamping non-finite or negative prices to `0.0`). When the provider echoes an authoritative `charged_amount_usd` that value wins; otherwise OpenHuman falls back to a static pricing catalog of known models. Usage is bucketed in UTC, keyed by model, with the **provider** derived from the `provider/model` prefix. All-zero usage payloads are skipped so providers that don't report usage don't inflate the request count.

### Budgets & enforcement

Budget enforcement is configured under the `[cost]` config block:

| Setting | Default | Role |
| --- | --- | --- |
| `enabled` | `true` | Gates **enforcement only**, not telemetry |
| `daily_limit_usd` | `10.00` | Hard daily cap |
| `monthly_limit_usd` | `100.00` | Hard monthly cap |
| `warn_at_percent` | `80` | Warn threshold for `check_budget` |

`check_budget` returns `Allowed`, `Warning` (warn threshold reached) or `Exceeded` (over the daily or monthly cap). A crucial detail: **`enabled` controls enforcement, not capture.** When it is `false`, `check_budget` always returns `Allowed` and hard caps are off. The agent still records usage unconditionally, so your spend history accumulates and you can review it *before* opting into hard caps. To hide the panel set `dashboard.enabled = false`; to clear history delete the JSONL file (it is local and never leaves the workspace).

### The 7-day dashboard

Settings → **Usage & Limits** hosts the cost dashboard (alongside background-activity controls). It renders a 7-day daily history (gap days zero-filled, oldest first), a token-usage chart, a monthly-pace projection, budget utilisation and a per-model cost breakdown. Dashboard colour-coding uses fractions of the monthly budget: bars flip to amber at the `warn_threshold` (default `0.8`) and red at the `alert_threshold` (default `0.95`). `budget_utilization` is clamped to `1.0` for display, while status is computed from the raw value. The panel polls roughly every 10 seconds and shows an "Updated Ns ago" freshness pill. A read-only fallback tracker (sharing the same JSONL file) serves the UI when the global tracker isn't yet initialised.

### RPC surface

Namespace `cost`, exposed as `openhuman.cost_*`:

| Method | Inputs | Output |
| --- | --- | --- |
| `cost_get_dashboard` | none | 7-day buckets, summary metrics, budget utilisation/status, per-model breakdown |
| `cost_get_daily_history` | `days?` (default 7, clamped 1 to 366) | Ordered daily entries, oldest first, gaps zero-filled |
| `cost_get_summary` | none | Live session / daily / monthly cost summary |

These are also exposed as read-only, default-ON agent tools so the agent can inspect its own spend.

***

## Cost & token compression

Because cost tracks **real token counts**, anything that shrinks the prompt directly lowers spend. OpenHuman's [TokenJuice token compression](token-compression.md) reduces the tokens sent on each call, and [model routing](model-routing/README.md) sends work to the cheapest model that can handle it. Both show up as lower bars in the dashboard and slower budget burn.

***

## See also

- [Token compression (TokenJuice)](token-compression.md)
- [Model routing](model-routing/README.md)
