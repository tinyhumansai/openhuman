---
description: >-
  Plans, credits and saved cards paid through Stripe or Coinbase, plus a local
  dashboard for token usage and cost estimates.
icon: credit-card
---

# Billing, cost and usage

OpenHuman keeps two separate ledgers. Billing is what you pay the hosted backend: plans, credit top-ups, saved cards and coupons, settled through Stripe or Coinbase. Cost and usage is what the agent spends on your behalf, tracked locally for each provider call so you can see estimated token costs.

Billing lives in the cloud. Cost and usage never leaves your workspace.

## Billing and payments

The `billing` domain holds no payment logic or state of its own. Each operation forwards an authenticated HTTPS call to the hosted backend (`/payments/*`, `/coupons/*`) using your stored app session and returns the backend's JSON response as is. The backend enforces authorization, plan ownership and payment policy. A missing or invalid session gets the backend's `401` or `403` directly. Tokens and card data are never logged.

Before sending anything, the adapter checks only the basics: plan, coupon and payment-method ids are not empty, `amountUsd` is a positive number, and the gateway is `stripe` or `coinbase`.

### Plans

There are three tiers, each with a monthly and an annual price.

| Tier  | Monthly | Annual    | Per-call discount vs pay-as-you-go |
| ----- | ------- | --------- | ---------------------------------- |
| Free  | $0      | $0        | None (the pay-as-you-go baseline)  |
| Basic | $19.99  | $199      | 50% cheaper per call               |
| Pro   | $199.99 | $1,799.99 | 90% cheaper per call               |

Higher tiers do not unlock features. Every tier has access to everything. You are buying cheaper inference.

### Payment providers

Two gateways are supported:

- Stripe handles plan purchases (Checkout sessions), the customer billing portal, credit top-ups, saved cards (SetupIntents) and auto-recharge.
- Coinbase Commerce handles crypto charges for credit top-ups and annual billing.

`top_up_credits` and `create_coinbase_charge` default to the `stripe` gateway and the `annual` interval. An empty gateway becomes Stripe.

### Credits, top-ups and auto-recharge

Besides a subscription, you can hold a USD credit balance. You can read the balance, page through transaction history, and top up through either gateway. Auto-recharge (Stripe only) refills credits from a saved card when the balance runs low. You can read and change its settings, and list, add, update and delete saved cards. Adding a card creates a Stripe SetupIntent. Deleting one is treated as a dangerous operation.

### Coupons

You can redeem a coupon code (`POST /coupons/redeem`) and list the coupons on your account (`GET /coupons/me`).

### Where billing lives in the app

The **Billing** button on the desktop Accounts page opens the hosted billing dashboard. There you manage plans, top-ups, coupons, cards and invoices. Other clients can reach the same operations through the RPC controllers below. They are not exposed as agent tools.

### RPC surface

The namespace is `billing`, exposed as `openhuman.billing_*`.

| Area | Methods |
| --- | --- |
| Plan and summary | `billing_get_summary`, `billing_get_current_plan`, `billing_purchase_plan`, `billing_create_portal_session` |
| Credits | `billing_get_balance`, `billing_top_up`, `billing_create_coinbase_charge`, `billing_get_transactions` |
| Auto-recharge | `billing_get_auto_recharge`, `billing_update_auto_recharge` |
| Saved cards | `billing_get_cards`, `billing_create_setup_intent`, `billing_update_card`, `billing_delete_card` |
| Coupons | `billing_redeem_coupon`, `billing_get_coupons` |

## Cost and usage dashboard

The `cost` domain is entirely local. It records the token usage and USD cost of every provider call to an append-only file (`<workspace>/state/costs.jsonl`), keeps daily and monthly totals in memory, and serves a 7-day dashboard over JSON-RPC. One tracker is shared by the agent loop and the dashboard, so each call is saved exactly once.

### How cost is computed

Cost per call comes from token counts and per-million-token prices. Non-finite or negative prices count as zero. If the provider reports the amount it charged (`charged_amount_usd`), that value wins. Otherwise OpenHuman uses a built-in price list of known models. Usage is bucketed by UTC day and keyed by model, and the provider comes from the `provider/model` prefix. Calls with all-zero usage are skipped, so providers that do not report usage do not inflate the request count.

### Usage views

The `[cost] monthly_limit_usd` setting is a display target, not a cap. The core does not refuse requests when estimated cost passes it. The backend enforces hosted credit limits separately, so the Usage view shows no budget gauge or limit status.

**Settings > Usage** shows the last seven UTC days of recorded cost and token activity, a projection of the month's pace, and a per-model breakdown. Costs are estimates when the provider did not return a charged amount. The dashboard refreshes about every 10 seconds.

The **Usage log** tab lists local provider-call records for a rolling period. It loads at most the 1,000 newest rows and lets you filter them by category, provider, cost source, and model or session. The row count and cost shown cover the filtered rows that were loaded, not a full billing total.

### RPC surface

The namespace is `cost`, exposed as `openhuman.cost_*`.

| Method                   | Inputs                                | Output                                                                    |
| ------------------------ | ------------------------------------- | ------------------------------------------------------------------------- |
| `cost_get_dashboard`     | none                                  | 7-day buckets, summary metrics, budget fields, per-model breakdown        |
| `cost_get_daily_history` | `days?` (default 7, clamped 1 to 366) | Daily entries, oldest first, gaps filled with zero                        |
| `cost_get_summary`       | none                                  | Live session, daily and monthly cost summary                              |
| `cost_get_usage_log`     | `days?`, `limit?`                     | Newest local provider-call records and category totals, up to 1,000 rows  |

These are also read-only agent tools, on by default, so the agent can check its own spend.

## Lowering cost

Cost follows real token counts, so anything that shrinks the prompt lowers spend. [Token compression](token-compression.md) cuts the tokens sent on each call, and [model routing](model-routing/README.md) sends work to the cheapest model that can handle it. Both show up as lower costs in the dashboard.

## See also

- [Token compression](token-compression.md): fewer tokens on each call.
- [Model routing](model-routing/README.md): the cheapest capable model for each task.
- [Privacy and security](privacy-and-security.md): what is logged, and what never is.
