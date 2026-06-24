/**
 * Classifier for user-actionable runtime errors (#3931).
 *
 * Maps an error *signal* (the user-facing message + error type that already
 * flow through the chat runtime / RPC layers) to a typed {@link UserErrorDescriptor}.
 * Only the two #3913 expected-user-states are recognised in this first slice:
 *
 *   - `budget_exceeded`      — managed backend 400 / `USER_INSUFFICIENT_CREDITS`
 *                              ("Insufficient budget")
 *   - `insufficient_credits` — BYO provider 402 / OpenRouter out of balance
 *                              ("requires more credits")
 *
 * Anything else returns `null` (NOT user-actionable → stays in normal error
 * flow / Sentry). This is deliberately conservative: a generic error must never
 * be promoted into the panel.
 *
 * NOTE: matching raw text here is a bootstrap. The intended end state is core
 * emitting a structured kind so the app does not pattern-match prose — see the
 * follow-ups in the #3931 PR. Keeping the rules in this one pure module makes
 * that migration a drop-in.
 */
import type { UserErrorDescriptor, UserErrorScope } from '../../types/userError';

export interface RuntimeErrorSignal {
  /** User-facing message produced upstream (e.g. chat `event.message`). */
  message?: string | null;
  /** Coarse error type/code when available (e.g. chat `event.error_type`). */
  errorType?: string | null;
  /** Where the signal came from; defaults to `chat`. */
  scope?: UserErrorScope;
  /** Originating core domain (metadata only). */
  sourceDomain?: string;
  /** Provider slug when known (metadata only, never secrets). */
  provider?: string;
}

/** Build the stable dedupe identity for an error. */
export function userErrorId(
  kind: UserErrorDescriptor['kind'],
  scope: UserErrorScope,
  provider?: string
): string {
  return `${kind}:${scope}:${provider ?? 'unknown'}`;
}

function haystack(signal: RuntimeErrorSignal): string {
  return `${signal.message ?? ''}\n${signal.errorType ?? ''}`.toLowerCase();
}

/**
 * Classify a runtime error signal. Returns a descriptor for a recognised
 * user-actionable state, or `null` when the error is not one.
 */
export function classifyUserActionableError(
  signal: RuntimeErrorSignal
): UserErrorDescriptor | null {
  const text = haystack(signal);
  if (!text.trim()) return null;
  const scope: UserErrorScope = signal.scope ?? 'chat';

  // Managed-budget exhaustion first: "insufficient budget" contains the word
  // "insufficient", so it must win over the BYO-credits rule below.
  const isBudget =
    text.includes('user_insufficient_credits') ||
    text.includes('insufficient budget') ||
    text.includes('budget_exceeded') ||
    text.includes('managed budget');
  if (isBudget) {
    return {
      id: userErrorId('budget_exceeded', scope, signal.provider),
      kind: 'budget_exceeded',
      severity: 'warning',
      scope,
      sourceDomain: signal.sourceDomain,
      provider: signal.provider,
      titleKey: 'userErrors.budgetExceeded.title',
      bodyKey: 'userErrors.budgetExceeded.body',
      action: 'open_billing',
    };
  }

  // BYO provider out of credits (OpenRouter 402, "requires more credits", etc).
  const isCredits =
    text.includes('requires more credits') ||
    text.includes('out of balance') ||
    text.includes('insufficient credits') ||
    text.includes('insufficient_credits') ||
    (text.includes('402') && text.includes('credit'));
  if (isCredits) {
    return {
      id: userErrorId('insufficient_credits', scope, signal.provider),
      kind: 'insufficient_credits',
      severity: 'warning',
      scope,
      sourceDomain: signal.sourceDomain,
      provider: signal.provider,
      titleKey: 'userErrors.insufficientCredits.title',
      bodyKey: 'userErrors.insufficientCredits.body',
      action: 'open_provider_settings',
    };
  }

  return null;
}
