import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { ExternalTransferPendingEvent } from '../../services/chatService';
import { callCoreRpc } from '../../services/coreRpcClient';
import privacyReducer, {
  clearActiveExternalForThread,
  clearDisclosuresForThread,
  disclosureFromEvent,
  dismissDisclosureForThread,
  hydratePrivacyMode,
  type PrivacyDisclosure,
  pushDisclosureForThread,
  setPrivacyMode,
} from '../privacySlice';
import { resetUserScopedState } from '../resetActions';

vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const EVENT: ExternalTransferPendingEvent = {
  thread_id: 'thread-1',
  provider_slug: 'openai',
  service: 'OpenAI',
  is_external: true,
  reason: 'inference',
  data_kinds: ['prompt'],
  risk_level: 'unknown',
  risk_categories: [],
};

function initial() {
  return privacyReducer(undefined, { type: '@@INIT' });
}

describe('privacySlice — reducers', () => {
  it('has the expected initial state', () => {
    expect(initial()).toEqual({
      privacyMode: null,
      disclosuresByThread: {},
      activeExternalByThread: {},
    });
  });

  it('setPrivacyMode updates the mode', () => {
    const state = privacyReducer(initial(), setPrivacyMode('local_only'));
    expect(state.privacyMode).toBe('local_only');
  });

  it('pushDisclosureForThread appends per thread', () => {
    const d = disclosureFromEvent(EVENT);
    const state = privacyReducer(
      initial(),
      pushDisclosureForThread({ threadId: 'thread-1', disclosure: d })
    );
    expect(state.disclosuresByThread['thread-1']).toHaveLength(1);
    expect(state.disclosuresByThread['thread-1'][0]).toMatchObject({
      providerSlug: 'openai',
      service: 'OpenAI',
      reason: 'inference',
      dataKinds: ['prompt'],
      isExternal: true,
    });
  });

  it('caps the per-thread ledger at 20 entries (drops oldest)', () => {
    let state = initial();
    let firstId = '';
    for (let i = 0; i < 25; i += 1) {
      const d = disclosureFromEvent(EVENT);
      if (i === 0) firstId = d.id;
      state = privacyReducer(state, pushDisclosureForThread({ threadId: 't', disclosure: d }));
    }
    const list = state.disclosuresByThread['t'];
    expect(list).toHaveLength(20);
    // The very first (oldest) disclosure was evicted.
    expect(list.some(entry => entry.id === firstId)).toBe(false);
  });

  it('dismissDisclosureForThread removes one entry by id', () => {
    const a = disclosureFromEvent(EVENT);
    const b = disclosureFromEvent(EVENT);
    let state = privacyReducer(
      initial(),
      pushDisclosureForThread({ threadId: 'thread-1', disclosure: a })
    );
    state = privacyReducer(state, pushDisclosureForThread({ threadId: 'thread-1', disclosure: b }));
    state = privacyReducer(state, dismissDisclosureForThread({ threadId: 'thread-1', id: a.id }));
    expect(state.disclosuresByThread['thread-1']).toHaveLength(1);
    expect(state.disclosuresByThread['thread-1'][0].id).toBe(b.id);
  });

  it('dismissing the last entry removes the thread key entirely', () => {
    const a = disclosureFromEvent(EVENT);
    let state = privacyReducer(
      initial(),
      pushDisclosureForThread({ threadId: 'thread-1', disclosure: a })
    );
    state = privacyReducer(state, dismissDisclosureForThread({ threadId: 'thread-1', id: a.id }));
    expect(state.disclosuresByThread['thread-1']).toBeUndefined();
  });

  it('clearDisclosuresForThread drops all disclosures for a thread', () => {
    const a = disclosureFromEvent(EVENT);
    let state = privacyReducer(
      initial(),
      pushDisclosureForThread({ threadId: 'thread-1', disclosure: a })
    );
    state = privacyReducer(state, clearDisclosuresForThread({ threadId: 'thread-1' }));
    expect(state.disclosuresByThread['thread-1']).toBeUndefined();
  });

  it('resetUserScopedState wipes disclosures, mode, and the active-external flags', () => {
    let state = privacyReducer(initial(), setPrivacyMode('standard'));
    state = privacyReducer(
      state,
      pushDisclosureForThread({ threadId: 't', disclosure: disclosureFromEvent(EVENT) })
    );
    expect(state.activeExternalByThread['t']).toBe(true);
    state = privacyReducer(state, resetUserScopedState());
    expect(state).toEqual({
      privacyMode: null,
      disclosuresByThread: {},
      activeExternalByThread: {},
    });
  });
});

describe('privacySlice — active external-transfer flag (#4437 finding 1)', () => {
  it('pushing an external disclosure marks the thread active-external', () => {
    const state = privacyReducer(
      initial(),
      pushDisclosureForThread({ threadId: 'thread-1', disclosure: disclosureFromEvent(EVENT) })
    );
    expect(state.activeExternalByThread['thread-1']).toBe(true);
  });

  it('a non-external disclosure does NOT mark the thread active-external', () => {
    const localEvent: ExternalTransferPendingEvent = { ...EVENT, is_external: false };
    const state = privacyReducer(
      initial(),
      pushDisclosureForThread({ threadId: 'thread-1', disclosure: disclosureFromEvent(localEvent) })
    );
    expect(state.activeExternalByThread['thread-1']).toBeUndefined();
  });

  // Finding 1a: dismissing the card must NOT flip the pill off while the
  // transfer is still active — the active flag survives dismissal.
  it('dismissing the disclosure card leaves the active-external flag set', () => {
    const d = disclosureFromEvent(EVENT);
    let state = privacyReducer(
      initial(),
      pushDisclosureForThread({ threadId: 'thread-1', disclosure: d })
    );
    state = privacyReducer(state, dismissDisclosureForThread({ threadId: 'thread-1', id: d.id }));
    // Ledger entry gone…
    expect(state.disclosuresByThread['thread-1']).toBeUndefined();
    // …but the live transfer flag remains until the turn boundary clears it.
    expect(state.activeExternalByThread['thread-1']).toBe(true);
  });

  // Finding 1b: the turn boundary clears the active flag even though the
  // (un-dismissed) ledger entry is still there for the card's history.
  it('clearActiveExternalForThread clears the flag but keeps the ledger', () => {
    const d = disclosureFromEvent(EVENT);
    let state = privacyReducer(
      initial(),
      pushDisclosureForThread({ threadId: 'thread-1', disclosure: d })
    );
    state = privacyReducer(state, clearActiveExternalForThread({ threadId: 'thread-1' }));
    expect(state.activeExternalByThread['thread-1']).toBeUndefined();
    // The disclosure history is untouched — only the live flag was cleared.
    expect(state.disclosuresByThread['thread-1']).toHaveLength(1);
  });

  it('clearActiveExternalForThread is a no-op for an unknown thread', () => {
    const state = privacyReducer(initial(), clearActiveExternalForThread({ threadId: 'nope' }));
    expect(state.activeExternalByThread).toEqual({});
  });
});

describe('privacySlice — disclosureFromEvent', () => {
  it('maps wire snake_case fields onto the disclosure and assigns unique ids', () => {
    const a = disclosureFromEvent(EVENT);
    const b = disclosureFromEvent(EVENT);
    expect(a.id).not.toBe(b.id);
    expect(a).toMatchObject({
      providerSlug: 'openai',
      service: 'OpenAI',
      isExternal: true,
      reason: 'inference',
      dataKinds: ['prompt'],
      riskLevel: 'unknown',
      riskCategories: [],
    });
    expect(typeof a.receivedAt).toBe('number');
  });
});

describe('privacySlice — hydratePrivacyMode thunk', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('sets the mode from the double-wrapped RPC result on success', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ result: { mode: 'sensitive' } } as never);
    const action = await hydratePrivacyMode()(vi.fn(), vi.fn(), undefined);
    expect(action.payload).toBe('sensitive');

    const state = privacyReducer(initial(), action as never);
    expect(state.privacyMode).toBe('sensitive');
  });

  it('resolves to null (and leaves mode untouched) on RPC failure', async () => {
    vi.mocked(callCoreRpc).mockRejectedValueOnce(new Error('core down'));
    const action = await hydratePrivacyMode()(vi.fn(), vi.fn(), undefined);
    expect(action.payload).toBeNull();

    const seeded = privacyReducer(initial(), setPrivacyMode('standard'));
    const state = privacyReducer(seeded, action as never);
    // Null payload must not clobber an already-known mode.
    expect(state.privacyMode).toBe('standard');
  });
});

// Type guard: exported PrivacyDisclosure stays structurally stable.
const _typecheck: PrivacyDisclosure = disclosureFromEvent(EVENT);
void _typecheck;
