# Ledger UX — asset filter, direction filter, copy tx-id (Tiny Place §4)

Sub-slice of the Tiny Place audit epic **#4776 §4 (Ledger)**. Pagination already
landed; this adds the remaining §4 UX gaps: **asset filter**, **in/out
direction filter**, and **copy transaction ID**. Frontend only.

## Problem

`LedgerSection` renders the public transaction ledger with pagination, but:

- No way to filter by asset (USDC / SOL / …).
- No way to see only incoming vs outgoing transactions.
- The Tx ID is plain text with no copy affordance (only a "View on chain"
  explorer link).

## Solution

All three are client-side, over the already-loaded `transactions`. Server-side
filtering is deliberately **not** used:

- `tx.asset` may be a **symbol or a mint address** (`assets.resolveAssetSymbol`
  resolves both), so a server-side `asset` filter value can't reliably match the
  stored form.
- Direction is **viewer-relative** (from/to vs the wallet address) — not a
  server concept for a public ledger.

### Copy tx-id

A copy button beside the Tx ID in the expanded detail. `navigator.clipboard
.writeText(tx.txId)`, transient "Copied" state (~1.5s), i18n label. Degrades
silently if the clipboard API is unavailable.

### Asset filter (client-side)

A `<select>` of `All` + the distinct `resolveAssetSymbol(tx.asset)` values found
in the loaded rows. Reusing the display-side resolver guarantees the option
labels match what rows show (no symbol/mint mismatch). Selecting one narrows the
rendered list.

### Direction filter (client-side, wallet-gated)

Segmented `All / In / Out`. `myAddr` comes from `fetchWalletStatus()` (the
Solana account address). A tx is **In** when `to === myAddr`, **Out** when
`from === myAddr`. The control is **hidden** when no wallet address is available
(the public ledger has no viewer-relative direction without one).

### Interaction

Both filters apply to the loaded `transactions` before render (memoized).
"Load more" continues to fetch raw pages; filters re-apply to the larger set.
When filters hide every loaded row, a distinct "no transactions match these
filters" message renders (vs. the empty-ledger copy). This is an explicit,
honest tradeoff: filters act on loaded rows, not the entire server ledger.

## Out of scope

- Wallet-balance-on-ledger, real-time updates, CSV export (other §4 items).
- Server-side filtering.

## Tests (Vitest, TDD)

Extend `LedgerSection.test.tsx`:

1. Copy button writes `tx.txId` to the clipboard and shows "Copied".
2. Asset filter narrows the list to the chosen asset.
3. Direction filter (with a mocked wallet) shows only incoming / outgoing rows.
4. Direction control is absent when `fetchWalletStatus` returns no address.
5. Filtering to zero rows renders the "no match" message, not the empty-ledger
   copy, and keeps the filter controls visible.

## i18n

New keys in `en.ts` + all 13 locales (no em dashes): `filterAsset`,
`filterAllAssets`, `direction`, `directionAll`, `directionIn`, `directionOut`,
`copyTxId`, `copied`, `noMatch`, `noMatchHint`.

## Debug logging

Reuse `debug('agentworld:ledger')`; log filter changes (asset symbol chosen,
direction) and copy outcomes — no PII (addresses already abbreviated in logs).
