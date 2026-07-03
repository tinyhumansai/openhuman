# 02.2 — Retry/fallback ownership → RunPolicy

`provider/reliable.rs` (1215 lines + 1443 tests) wraps several production
provider paths in retry/fallback. The crate loop is currently pinned to a single
attempt (`RunPolicy.retry.max_attempts = 1`) to avoid double-retry while
OpenHuman owns reliability. Audit then collapse to one owner: the SDK.

Current status (2026-07-02): do not delete `ReliableProvider` yet. Provider
factory paths still wrap the OpenHuman backend in `ReliableProvider` for
configured retries and model fallbacks, and `run_policy_for` explicitly sets
`policy.retry.max_attempts = 1` so TinyAgents does not double-retry while that
wrapper owns reliability. Deletion is blocked on moving the configured fallback
chain into registered crate model routes plus event-visible retry/fallback
parity. TinyAgents 1.3.0 `RunPolicy::fallback` can retry fallback model routes,
but it does not by itself emit the OpenHuman `FallbackSelected` parity signal;
that event parity requires `ModelFallbackMiddleware` or an equivalent bridge.

## Steps

1. Map `ReliableProvider` behaviors onto crate primitives:
   transient 429/5xx → `RetryPolicy` (exp backoff) + `RateLimitMiddleware`;
   provider chain → `FallbackPolicy` (ordered model names, needs 02.1
   multi-registration); permanent config rejection / billing exhaustion →
   non-retryable `ProviderError { retryable: false }` mapping in
   `ProviderModel`.
2. Keep OpenHuman's error classification (`ops/http_error.rs`,
   `billing_error`, `auth_error_registry`) as the mapper that fills
   `ProviderError`; it is product knowledge.
3. Un-wrap `ReliableProvider` from `session/builder/factory.rs` (re-layered
   there 2026-06-30) once parity tests cover: transient retry count, retry
   events (`RetryScheduled`), fallback events (`FallbackSelected`), and NO
   double-retry (assert total attempt counts with `MockModel::call_count`).
   Keep `RunPolicy.retry.max_attempts = 1` until this swap happens; raising it
   earlier would reintroduce double-retry on top of `ReliableProvider`.
4. Non-turn callers of `ReliableProvider` and its classifier helpers (memory-tree
   local summarizer, memory scoring, triage error classification): either route
   them through a minimal harness invoke or a small shared retry/classification
   utility — inventory call sites first; do not leave a fork.

## Deletions

- `src/openhuman/inference/provider/reliable.rs` + tests (rewrite the
  behaviorally-relevant cases against RunPolicy).

## Acceptance

- Attempt-count parity matrix (429, 500, config rejection, billing) green.
- `FallbackSelected`/`RetryScheduled` visible in the event bridge.
