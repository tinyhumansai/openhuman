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

/** Stable discriminator the UI branches on. Extend as new states are added. */
export type UserErrorKind = 'insufficient_credits' | 'budget_exceeded';

/** Where the failure originated, for grouping/labelling (privacy-safe). */
export type UserErrorScope = 'chat' | 'cron' | 'provider' | 'integration' | 'workspace';

/** Primary next-step the user can take. `dismiss` is always available too. */
export type UserErrorAction = 'open_billing' | 'open_provider_settings' | 'dismiss';

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
