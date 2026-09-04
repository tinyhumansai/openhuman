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

/**
 * The durable form of the corrupt-store notice (openhuman#5820).
 *
 * Same kind, scope and therefore the same descriptor id as the live
 * `memory_store_corrupt` socket broadcast, so the two paths collapse into
 * one NoticeCenter entry: the socket reaches a connected renderer instantly,
 * and the status poll replays it for a renderer that was not connected when
 * the quarantine happened (a boot-time integrity check). `null` once the
 * user has re-synced — the caller resolves the entry then.
 */
export function classifyMemoryQuarantine(
  quarantine: { resynced: boolean } | null | undefined
): UserErrorDescriptor | null {
  if (!quarantine || quarantine.resynced) return null;
  return {
    id: userErrorId('memory_store_corrupt', 'memory'),
    kind: 'memory_store_corrupt',
    severity: 'error',
    scope: 'memory',
    sourceDomain: 'memory',
    titleKey: 'userErrors.memoryStoreCorrupt.title',
    bodyKey: 'userErrors.memoryStoreCorrupt.body',
    action: 'open_memory_sync',
  };
}

/**
 * #5324: the memory pipeline's typed `budget_exhausted` cause, promoted to a
 * first-class user-actionable error.
 *
 * Unlike the classifiers below this takes the core's stable `FailureCode`
 * directly rather than pattern-matching prose — the memory pipeline already
 * emits a typed cause on `first_blocking_cause`, so there is nothing to guess.
 * That is the end state the text matchers below are migrating toward.
 *
 * Scoped to `workspace` (not `chat`) so a memory outage and a chat outage
 * dedupe as separate entries. They have different fixes, and a user can hit
 * both at once off the same exhausted budget.
 *
 * @param failureCode The `first_blocking_cause.code` from
 *   `memory_tree_pipeline_status`.
 * @returns A descriptor when the cause is user-actionable, else `null`.
 */
export function classifyMemoryPipelineFailure(
  failureCode: string | null | undefined
): UserErrorDescriptor | null {
  if (failureCode !== 'budget_exhausted') return null;
  return {
    id: userErrorId('memory_budget_exhausted', 'workspace'),
    kind: 'memory_budget_exhausted',
    severity: 'warning',
    scope: 'workspace',
    sourceDomain: 'memory_tree',
    titleKey: 'userErrors.memoryBudgetExhausted.title',
    bodyKey: 'userErrors.memoryBudgetExhausted.body',
    action: 'open_embeddings_settings',
  };
}

/**
 * A third-party integration is failing, so the connection state on screen is
 * stale (#composio).
 *
 * Unlike the matchers below this does not sniff the prose: the caller already
 * knows the integration errored, and the message is the backend's own
 * user-facing explanation. It rides on `detail` verbatim rather than being
 * pattern-matched into a billing entry — "your credits are exhausted" and
 * "your connected tools have silently stopped" are different things to tell
 * someone, and only the second explains why their agent stopped acting.
 *
 * @param message The integration client's user-facing error text.
 * @param provider Integration slug (e.g. `composio`) — metadata only.
 */
export function classifyIntegrationError(
  message: string | null | undefined,
  provider: string
): UserErrorDescriptor | null {
  const detail = message?.trim();
  if (!detail) return null;
  return {
    id: userErrorId('integration_degraded', 'integration', provider),
    kind: 'integration_degraded',
    severity: 'warning',
    scope: 'integration',
    sourceDomain: provider,
    provider,
    titleKey: 'userErrors.integrationDegraded.title',
    bodyKey: 'userErrors.integrationDegraded.body',
    detail,
    action: 'open_connections',
  };
}

/**
 * #6034: a completed reply that reached neither writer, so there is nothing to
 * render and nothing to re-read.
 *
 * The caller already knows this happened — it just failed an append and a
 * refetch — so, like the two constructors above, this takes the fact rather
 * than sniffing prose. Scoped per thread so two different lost replies are two
 * entries, and one thread failing repeatedly is one entry with a count.
 */
export function classifyReplyDeliveryFailure(threadId: string): UserErrorDescriptor | null {
  const thread = threadId?.trim();
  if (!thread) return null;
  return {
    id: userErrorId('reply_delivery_failed', 'chat', thread),
    kind: 'reply_delivery_failed',
    severity: 'error',
    scope: 'chat',
    sourceDomain: 'chat',
    titleKey: 'userErrors.replyDeliveryFailed.title',
    bodyKey: 'userErrors.replyDeliveryFailed.body',
    action: 'dismiss',
  };
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

  // Provider configured but no API key set — a deterministic credential-guard
  // failure (no HTTP). Matches the stable `api_key_missing` kind token emitted
  // by core (e.g. cron `user_error`) AND the verbatim guard prose, mirroring
  // the Rust single-source matcher `is_api_key_unset_message` (observability.rs)
  // so a wording drift on either side fails its own test rather than silently
  // dropping the signal (TAURI-RUST-HCK / #4165).
  const isApiKeyMissing =
    text.includes('api_key_missing') ||
    text.includes('api key not set') ||
    text.includes('missing api key') ||
    text.includes('no api key is configured') ||
    text.includes('no api key supplied');
  if (isApiKeyMissing) {
    return {
      id: userErrorId('api_key_missing', scope, signal.provider),
      kind: 'api_key_missing',
      severity: 'warning',
      scope,
      sourceDomain: signal.sourceDomain,
      provider: signal.provider,
      titleKey: 'userErrors.apiKeyMissing.title',
      bodyKey: 'userErrors.apiKeyMissing.body',
      action: 'open_provider_settings',
    };
  }

  // The memory-tree store was corrupt and has been quarantined + rebuilt
  // empty (openhuman#5820). Token-only on purpose: the only producer is the
  // core's `STORE_CORRUPT_KIND` broadcast, and the underlying SQLite prose
  // ("database disk image is malformed") also appears in raw logs other
  // domains relay — promoting prose here could turn an unrelated relay into
  // a "your memory was quarantined" panel entry.
  if (text.includes('memory_store_corrupt')) {
    return {
      id: userErrorId('memory_store_corrupt', scope, signal.provider),
      kind: 'memory_store_corrupt',
      severity: 'error',
      scope,
      sourceDomain: signal.sourceDomain,
      provider: signal.provider,
      titleKey: 'userErrors.memoryStoreCorrupt.title',
      bodyKey: 'userErrors.memoryStoreCorrupt.body',
      // Re-syncing sources is the remediation — the rebuilt store is empty
      // and repopulates from there.
      action: 'open_memory_sync',
    };
  }

  // The local model runtime a workload depends on is unusable — Ollama is not
  // running, or the configured model was never pulled (#5354). Emitted by core
  // as the stable `local_model_unavailable` kind token (memory embedder health
  // gate); the prose variants match the wording the local embedder itself
  // produces, so a signal that arrives with a message instead of a token still
  // classifies. Deliberately last: it is the narrowest rule, and a provider
  // that is out of credits should keep its billing remediation even when the
  // message happens to name Ollama.
  // Each prose matcher is anchored on the FULL producer wording, never a bare
  // `daemon unreachable at`: backend connection-health logs in other domains
  // emit that phrase too, and promoting one of those into an "install Ollama"
  // panel entry would be worse than showing nothing. Mirrors the same
  // deliberate anchoring in the Rust matcher `is_ollama_user_config_rejection`.
  const isLocalModelUnavailable =
    text.includes('local_model_unavailable') ||
    // tinyagents embedder, daemon not listening.
    text.includes('is ollama running') ||
    // tinyagents embedder, model never pulled — the second shape the Rust
    // classifier recognises, so the two sides stay symmetric.
    (text.includes('ollama embedding model') && text.includes('is not installed at')) ||
    // platform doctor report.
    text.includes('ollama daemon unreachable') ||
    // memory embedder health gate.
    text.includes('ollama embeddings opted-in but daemon unreachable at');
  if (isLocalModelUnavailable) {
    return {
      id: userErrorId('local_model_unavailable', scope, signal.provider),
      kind: 'local_model_unavailable',
      severity: 'warning',
      scope,
      sourceDomain: signal.sourceDomain,
      provider: signal.provider,
      titleKey: 'userErrors.localModelUnavailable.title',
      bodyKey: 'userErrors.localModelUnavailable.body',
      // Local AI + embedding provider selection both live behind
      // `/settings/llm`, which redirects to Connections → API keys.
      action: 'open_provider_settings',
    };
  }

  return null;
}
