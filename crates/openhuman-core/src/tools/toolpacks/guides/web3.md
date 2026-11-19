Wallet, swap, bridge, contract-call and x402 payment actions on the user's own wallet identities. These move real money: **read → quote → confirm → execute**, never fewer steps.

1. **Frame.** One line: asset, amount, chain, from, to. Ask once if chain, asset or recipient is ambiguous.
2. **Inspect.** `wallet_status`, `wallet_chain_status`, `wallet_balances` (a row not `ready` was unreadable). Never state a balance no tool returned.
3. **Quote.** `web3_swap_quote` (after `web3_swap_routes` if open), `web3_bridge_quote`, `web3_dapp_call`, or `wallet_prepare_transfer` (`amountRaw` atomic: lamports, 10^-6 USDC, wei). Flag odd fees, slippage or hops.
4. **Confirm.** In a chat, show source, full destination, asset, amount, fee, slippage and quote id, and get an explicit yes. `confirmed: true` is a machine gate, not a substitute for asking.
5. **Execute.** Only on that yes: `web3_swap_execute` / `web3_bridge_execute` / `web3_dapp_execute`, or `wallet_execute_prepared`, with this turn's quote id. Re-quote past ~60s. `wallet_execute_prepared` runs only within the owner's spend policy; if the asset is not allowed, tell the user to finish in the wallet UI.
6. **Track.** `wallet_tx_status` / `wallet_tx_receipt` / `wallet_lookup_tx` for a returned hash. Never invent a hash or link.

**Automated turns** (background job, approval-free workflow, or a scheduled agent that opted into `scheduled_actions`): prepare then `wallet_execute_prepared`; it runs only if the owner enabled automated transfers and the amount fits the caps (per transfer, rolling day, per recipient per week). A `WALLET_POLICY_REFUSED:` error names the cap: report and stop, never split a payment. Plain cron turns are read-only; only a cron job whose agent opted into scheduled actions may act.

`x402_request` pays an x402 API (HTTP 402) in USDC and retries with the proof. In a chat confirm the price first; automated turns pay only within the x402 budget.

Stop cleanly: missing identity or chain → **Settings → Recovery Phrase** or **Connections**; failed quote → give the reason, suggest the smallest fix, wait. Never retry a write: a failed execute may be on chain, so check `wallet_tx_status`. Never echo keys, seed phrases, signed payloads or raw RPC errors.
