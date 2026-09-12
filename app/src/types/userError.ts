/**
 * User-actionable runtime errors (#3931).
 *
 * A small, privacy-safe contract for *expected user states* that the app must
 * surface durably and actionably — e.g. a BYO provider being out of credits or
 * a managed budget being exhausted (the #3913 paths). These are deliberately
 * NOT Sentry-worthy crashes, but they must not vanish either: they land in a
 * first-class panel in the desktop shell with a clear next action.
 *
 * This is intentionally a thin, additive frontend contract for the first slice.
 * The structured Rust-core emission path (stable `message_key` + args emitted by
 * core rather than classified from text app-side) is the planned follow-up; this
 * type is shaped to accept that source without the panel changing.
 */

/**
 * Stable discriminator the UI branches on. Extend as new states are added.
 *
 * `memory_budget_exhausted` (#5324) is deliberately separate from
 * `budget_exceeded` even though both originate in the same managed cycle
 * budget: the consequence and the fix differ. Chat being gated is immediately
 * visible and is fixed by adding credits; memory silently stopping is
 * invisible and is fixed by pointing embeddings at local Ollama or a BYO key.
 * Collapsing them would send memory users to the billing screen, which does
 * not solve their problem.
 */
export type UserErrorKind =
  | 'insufficient_credits'
  | 'budget_exceeded'
  | 'api_key_missing'
  | 'memory_budget_exhausted'
  /**
   * The local model runtime a workload depends on is not usable — Ollama is
   * not running, or the configured model was never pulled (#5354). Mirrors the
   * core-side `LOCAL_MODEL_UNAVAILABLE_KIND` token.
   */
  | 'local_model_unavailable'
  /**
   * A third-party integration is answering with errors, so the connection
   * state the app is showing is stale (today: Composio). Distinct from
   * `budget_exceeded` even when the underlying cause is exhausted credits:
   * the consequence is "your tools are silently not running", and naming that
   * is the whole point — a billing-worded entry would not tell the user their
   * connections have stopped working.
   */
  | 'integration_degraded'
  /**
   * The memory-tree store was corrupt and has been quarantined + rebuilt
   * empty (openhuman#5820). Mirrors the core-side `STORE_CORRUPT_KIND`
   * token. The damaged file is preserved on disk beside the store; the
   * rebuilt tree repopulates by re-syncing sources, which is why the action
   * deep-links to Brain's sync tab rather than any settings screen.
   */
  | 'memory_store_corrupt'
  /**
   * A reply the agent finished could not be shown: neither the core's write
   * nor the client's append left a row, and re-reading the thread did not
   * find one (#6034). Distinct from every entry above because nothing is
   * misconfigured — the turn ran and was paid for, and the only useful next
   * step is to ask again, so the action is `dismiss` rather than a settings
   * deep link.
   */
  | 'reply_delivery_failed';

/** Where the failure originated, for grouping/labelling (privacy-safe). */
export type UserErrorScope =
  | 'chat'
  | 'cron'
  | 'provider'
  | 'integration'
  | 'workspace'
  /** Memory ingestion / embedding pipeline. */
  | 'memory';

/** Primary next-step the user can take. `dismiss` is always available too. */
export type UserErrorAction =
  | 'open_billing'
  | 'open_provider_settings'
  | 'open_embeddings_settings'
  /** The connections screen — where integration health is polled and re-read. */
  | 'open_connections'
  /** Brain's sync tab — where memory sources are re-synced after a store rebuild. */
  | 'open_memory_sync'
  | 'dismiss';

export type UserErrorSeverity = 'warning' | 'error';

/**
 * Classifier output: everything that identifies and presents an error, minus
 * the runtime bookkeeping (timestamps / counts) the store owns. Carries i18n
 * *keys* — never raw provider text — so all copy stays translatable.
 */
export interface UserErrorDescriptor {
  /** Stable dedupe identity (`kind:scope:provider`). */
  id: string;
  kind: UserErrorKind;
  severity: UserErrorSeverity;
  scope: UserErrorScope;
  /** Originating core domain/operation, metadata only (e.g. `chat`). */
  sourceDomain?: string;
  /** Provider slug when safe + useful (e.g. `openrouter`). Never secrets. */
  provider?: string;
  /** i18n key for the short title. */
  titleKey: string;
  /** i18n key for the one-line explanation. */
  bodyKey: string;
  /**
   * Extra already-user-facing detail from the source, shown verbatim beneath
   * the translated body.
   *
   * The rest of this contract is deliberately i18n *keys* so all copy stays
   * translatable, and that stays the rule. This is the narrow exception for a
   * message a backend wrote **for the user** and that the app cannot restate
   * without losing what actually went wrong (e.g. which integration call
   * failed and why). Never put raw provider responses, tokens, prompts or PII
   * here — only text already destined for the user's screen.
   */
  detail?: string;
  action: UserErrorAction;
}

/** A live entry in the panel: a descriptor plus store-owned bookkeeping. */
export interface UserActionableError extends UserErrorDescriptor {
  /** First time this entry was seen (epoch ms). */
  occurredAt: number;
  /** Most recent occurrence (epoch ms). */
  lastSeenAt: number;
  /** How many times this exact state has recurred while active. */
  count: number;
  /** Set when resolved/acted-on; resolved entries drop out of the active list. */
  resolvedAt?: number;
}
