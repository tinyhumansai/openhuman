import { describe, expect, it } from 'vitest';

import {
  classifyMemoryPipelineFailure,
  classifyMemoryQuarantine,
  classifyUserActionableError,
  userErrorId,
} from '../classify';

const BUDGET_MSG = 'OpenHuman API error (400): Insufficient budget';
const CREDITS_MSG = 'OpenRouter: this request requires more credits';
const BALANCE_MSG = 'HTTP 402: account is out of balance';
const GENERIC_MSG = 'Something went wrong. Please try again.';

describe('classifyUserActionableError', () => {
  it('classifies managed-budget exhaustion (USER_INSUFFICIENT_CREDITS)', () => {
    const a = classifyUserActionableError({ message: BUDGET_MSG });
    expect(a?.kind).toBe('budget_exceeded');
    expect(a?.action).toBe('open_billing');
    expect(a?.titleKey).toBe('userErrors.budgetExceeded.title');

    const b = classifyUserActionableError({ errorType: 'USER_INSUFFICIENT_CREDITS' });
    expect(b?.kind).toBe('budget_exceeded');
  });

  it('classifies BYO provider out-of-credits (402 / requires more credits)', () => {
    const a = classifyUserActionableError({ message: CREDITS_MSG });
    expect(a?.kind).toBe('insufficient_credits');
    expect(a?.action).toBe('open_provider_settings');
    expect(a?.titleKey).toBe('userErrors.insufficientCredits.title');

    const b = classifyUserActionableError({ message: BALANCE_MSG });
    expect(b?.kind).toBe('insufficient_credits');
  });

  it('prefers managed-budget over BYO-credits when text says "insufficient budget"', () => {
    // "insufficient budget" contains "insufficient" but must not be misread as
    // the BYO-credits case.
    const a = classifyUserActionableError({ message: 'insufficient budget for this request' });
    expect(a?.kind).toBe('budget_exceeded');
  });

  it('classifies a configured provider with no API key (cron user_error kind token)', () => {
    // Core emits the stable kind token in error_type — classify must accept it.
    const a = classifyUserActionableError({ errorType: 'api_key_missing', scope: 'cron' });
    expect(a?.kind).toBe('api_key_missing');
    expect(a?.scope).toBe('cron');
    expect(a?.action).toBe('open_provider_settings');
    expect(a?.titleKey).toBe('userErrors.apiKeyMissing.title');

    // …and the verbatim credential-guard prose (mirrors Rust is_api_key_unset_message).
    for (const msg of [
      'openrouter: API key not set',
      'Missing API key for provider',
      'No API key is configured',
      'no api key supplied',
    ]) {
      expect(classifyUserActionableError({ message: msg })?.kind).toBe('api_key_missing');
    }
  });

  it('does NOT classify an invalid/rejected API key (401) as missing-key', () => {
    // A present-but-rejected key is a different state — must not be promoted.
    expect(classifyUserActionableError({ message: 'Invalid API key (401)' })).toBeNull();
    expect(classifyUserActionableError({ message: 'Incorrect API key provided' })).toBeNull();
  });

  it('classifies an unusable local model runtime (memory user_error kind token)', () => {
    // Core's memory embedder health gate emits the stable kind token with
    // error_source=memory (#5354); socketService maps that to the memory scope.
    const a = classifyUserActionableError({
      errorType: 'local_model_unavailable',
      scope: 'memory',
      sourceDomain: 'memory',
    });
    expect(a?.kind).toBe('local_model_unavailable');
    expect(a?.scope).toBe('memory');
    expect(a?.action).toBe('open_provider_settings');
    expect(a?.titleKey).toBe('userErrors.localModelUnavailable.title');
    expect(a?.bodyKey).toBe('userErrors.localModelUnavailable.body');
    expect(a?.id).toBe(userErrorId('local_model_unavailable', 'memory', undefined));

    // …and every prose shape the local embedder / health gate / doctor
    // produce. Both Rust-side shapes are covered so the two classifiers stay
    // symmetric — daemon-not-listening AND model-never-pulled.
    for (const msg of [
      'ollama embed request failed (is Ollama running at http://localhost:11434?)',
      'Ollama embedding model `bge-m3` is not installed at http://localhost:11434. Run `ollama pull bge-m3`',
      'Ollama daemon unreachable at http://localhost:11434',
      'ollama embeddings opted-in but daemon unreachable at http://localhost:11434',
    ]) {
      expect(classifyUserActionableError({ message: msg })?.kind).toBe('local_model_unavailable');
    }
  });

  it('classifies a quarantined corrupt memory store (memory user_error kind token)', () => {
    // Core's shared corruption recovery emits the stable STORE_CORRUPT_KIND
    // token with error_source=memory after quarantine + rebuild
    // (openhuman#5820); the CTA routes to Brain's sync tab because the
    // rebuilt store repopulates by re-syncing sources.
    const a = classifyUserActionableError({
      errorType: 'memory_store_corrupt',
      scope: 'memory',
      sourceDomain: 'memory',
    });
    expect(a?.kind).toBe('memory_store_corrupt');
    expect(a?.severity).toBe('error');
    expect(a?.scope).toBe('memory');
    expect(a?.action).toBe('open_memory_sync');
    expect(a?.titleKey).toBe('userErrors.memoryStoreCorrupt.title');
    expect(a?.bodyKey).toBe('userErrors.memoryStoreCorrupt.body');
    expect(a?.id).toBe(userErrorId('memory_store_corrupt', 'memory', undefined));
  });

  it('does NOT promote raw SQLite corruption prose relayed by another domain', () => {
    // Token-only on purpose: "database disk image is malformed" appears in
    // raw logs other domains relay, and a relayed log line must not become a
    // "your memory was quarantined" panel entry.
    expect(classifyUserActionableError({ message: 'database disk image is malformed' })).toBeNull();
    expect(
      classifyUserActionableError({
        message: 'sync failed: file is not a database',
        scope: 'memory',
      })
    ).toBeNull();
  });

  it('does NOT promote a bare "daemon unreachable" from another domain', () => {
    // Backend connection-health logs use this phrase too. Matching it loosely
    // would tell a user with a flaky backend link to install Ollama. The Rust
    // matcher anchors on the full producer wording for the same reason.
    expect(
      classifyUserActionableError({ message: 'backend daemon unreachable at api.tinyhumans.ai' })
    ).toBeNull();
  });

  it('keeps billing remediation for a credits error that also names Ollama', () => {
    // The local-runtime rule is last on purpose: an out-of-credits provider
    // must not be told to install Ollama.
    const a = classifyUserActionableError({ message: 'ollama proxy requires more credits' });
    expect(a?.kind).toBe('insufficient_credits');
  });

  it('returns null for generic / non-actionable errors and empty input', () => {
    expect(classifyUserActionableError({ message: GENERIC_MSG })).toBeNull();
    expect(classifyUserActionableError({ message: '', errorType: 'inference' })).toBeNull();
    expect(classifyUserActionableError({})).toBeNull();
  });

  it('defaults scope to chat and carries provider into a stable dedupe id', () => {
    const a = classifyUserActionableError({
      message: 'requires more credits',
      provider: 'openrouter',
    });
    expect(a?.scope).toBe('chat');
    expect(a?.id).toBe(userErrorId('insufficient_credits', 'chat', 'openrouter'));
  });
});

// ── #5324: memory pipeline budget exhaustion ────────────────────────────────

describe('classifyMemoryQuarantine', () => {
  it('reports an outstanding quarantine under the same id as the socket path', () => {
    const fromPoll = classifyMemoryQuarantine({ resynced: false });
    const fromSocket = classifyUserActionableError({
      errorType: 'memory_store_corrupt',
      scope: 'memory',
      sourceDomain: 'memory',
    });
    expect(fromPoll?.kind).toBe('memory_store_corrupt');
    expect(fromPoll?.action).toBe('open_memory_sync');
    expect(fromPoll?.id).toBe(fromSocket?.id);
  });

  it('is null once the store has been re-synced, and for no quarantine at all', () => {
    expect(classifyMemoryQuarantine({ resynced: true })).toBeNull();
    expect(classifyMemoryQuarantine(null)).toBeNull();
    expect(classifyMemoryQuarantine(undefined)).toBeNull();
  });
});

describe('classifyMemoryPipelineFailure', () => {
  it('promotes a budget-exhausted memory pipeline to a user-actionable error', () => {
    const d = classifyMemoryPipelineFailure('budget_exhausted');
    expect(d).not.toBeNull();
    expect(d!.kind).toBe('memory_budget_exhausted');
    expect(d!.scope).toBe('workspace');
    expect(d!.sourceDomain).toBe('memory_tree');
  });

  it('routes the CTA to embeddings settings, not billing', () => {
    // Adding credits does not fix a memory outage — pointing embeddings at a
    // local or BYO provider does. Sending the user to billing would be a dead
    // end.
    expect(classifyMemoryPipelineFailure('budget_exhausted')!.action).toBe(
      'open_embeddings_settings'
    );
  });

  it('dedupes separately from the chat-scoped budget error', () => {
    // One exhausted budget can break both chat and memory at once; they need
    // different fixes, so they must not collapse into one panel entry.
    const memory = classifyMemoryPipelineFailure('budget_exhausted')!;
    const chat = classifyUserActionableError({ message: 'Insufficient budget' })!;
    expect(memory.id).not.toBe(chat.id);
  });

  it('ignores every other failure code', () => {
    for (const code of [
      'auth_missing',
      'auth_invalid',
      'embeddings_unconfigured',
      'embedding_dim_mismatch',
      'local_model_unavailable',
      'extraction_timeout',
      'storage_unavailable',
      'transient',
    ]) {
      expect(classifyMemoryPipelineFailure(code)).toBeNull();
    }
  });

  it('is null-safe for an absent cause', () => {
    expect(classifyMemoryPipelineFailure(null)).toBeNull();
    expect(classifyMemoryPipelineFailure(undefined)).toBeNull();
  });
});
