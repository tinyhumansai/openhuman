import { createAsyncThunk, createSlice, type PayloadAction } from '@reduxjs/toolkit';
import debug from 'debug';

import type {
  EgressDataKind,
  EgressReason,
  EgressRiskLevel,
  ExternalTransferPendingEvent,
} from '../services/chatService';
import { callCoreRpc } from '../services/coreRpcClient';
import { CORE_RPC_METHODS } from '../services/rpcMethods';
import { resetUserScopedState } from './resetActions';

const privacyLog = debug('privacy:slice');

/**
 * Privacy Mode values as serialized by the Rust core (snake_case). Mirrors the
 * `PrivacyMode` type in {@link ../components/settings/panels/PrivacyModeSection}.
 * The disclosure surface reads this to show the *current posture* alongside the
 * per-action egress state — it does NOT own the setting (that stays in
 * PrivacyModeSection); the slice is hydrated on boot and kept loosely in sync.
 */
export type PrivacyMode = 'local_only' | 'standard' | 'sensitive';

/**
 * One external-transfer disclosure projected onto a thread (#4437 / S3). Built
 * from an `external_transfer_pending` socket event. DISCLOSURE ONLY — there is
 * no approve/deny decision here (that is S4 #4438); the only user action is
 * dismissal.
 */
export interface PrivacyDisclosure {
  /** Client-generated id — the dismissal handle and React key. */
  id: string;
  /** Provider identifier (e.g. `openai`). Public, not PII. */
  providerSlug: string;
  /** Human-facing destination service (e.g. `OpenAI`, `Gmail`). */
  service: string;
  /** Whether the transfer leaves the device (always true in practice). */
  isExternal: boolean;
  /** Why the transfer is happening. */
  reason: EgressReason;
  /** Categories of user data being sent. */
  dataKinds: EgressDataKind[];
  /** Core-assigned risk grade. `"unknown"` today. */
  riskLevel: EgressRiskLevel;
  /** Named risk categories. Empty today. */
  riskCategories: string[];
  /** When the disclosure was received, milliseconds since epoch. */
  receivedAt: number;
}

interface PrivacyState {
  /**
   * Current data-egress posture. `null` until hydrated from the core (or if the
   * RPC fails). Kept for the persistent status pill; not authoritative — the
   * setting lives in the core and is edited via PrivacyModeSection.
   */
  privacyMode: PrivacyMode | null;
  /**
   * Per-thread disclosure ledger, newest last. The disclosure card renders the
   * most recent entry for the active thread; dismissal removes one entry by id.
   *
   * IMPORTANT: this ledger is user-DISMISSIBLE history for the in-chat card
   * only. It MUST NOT drive the status pill's on/off-device sub-state — a
   * dismissal removing the last entry would otherwise flip the pill to
   * "on-device" while the transfer is still in flight, and an un-dismissed
   * historical entry would keep it "off-device" during later purely-local
   * turns. The pill reads {@link activeExternalByThread} instead.
   */
  disclosuresByThread: Record<string, PrivacyDisclosure[]>;
  /**
   * Per-thread "an external transfer is active on the current turn" flag. Set
   * true when an external disclosure is pushed, and CLEARED on the turn
   * boundary (chat_done / chat_error / socket-disconnect reconcile) by
   * ChatRuntimeProvider — the same turn-completion signals the approval/plan
   * flows use. This is the SOLE source of truth for the pill's off-device
   * state, kept deliberately separate from the dismissible ledger above so the
   * pill reflects the live transfer, not the card's history.
   */
  activeExternalByThread: Record<string, boolean>;
}

const initialState: PrivacyState = {
  privacyMode: null,
  disclosuresByThread: {},
  activeExternalByThread: {},
};

/** Cap the per-thread ledger so a chatty turn can't grow it unbounded. */
const MAX_DISCLOSURES_PER_THREAD = 20;

let disclosureSeq = 0;

/**
 * Build a {@link PrivacyDisclosure} from a socket event. Exported so the store
 * wiring in {@link ../providers/ChatRuntimeProvider} and unit tests share one
 * mapping. `id` is a monotonically-increasing client id (stable within a
 * session, never sent anywhere).
 */
export function disclosureFromEvent(event: ExternalTransferPendingEvent): PrivacyDisclosure {
  disclosureSeq += 1;
  return {
    id: `disclosure-${disclosureSeq}`,
    providerSlug: event.provider_slug,
    service: event.service,
    isExternal: event.is_external,
    reason: event.reason,
    dataKinds: event.data_kinds,
    riskLevel: event.risk_level,
    riskCategories: event.risk_categories,
    receivedAt: Date.now(),
  };
}

/**
 * Hydrate the current Privacy Mode from the core on boot. Mirrors the RPC
 * PrivacyModeSection uses (`config_get_privacy_mode`) whose result is the
 * double-wrapped `{ result: { mode } }` shape. Failures resolve to `null` so
 * the pill degrades gracefully rather than throwing.
 */
export const hydratePrivacyMode = createAsyncThunk<PrivacyMode | null>(
  'privacy/hydratePrivacyMode',
  async () => {
    try {
      const resp = await callCoreRpc<{ result: { mode: PrivacyMode } }>({
        method: CORE_RPC_METHODS.configGetPrivacyMode,
        params: {},
      });
      privacyLog('[privacy] hydrated mode=%s', resp.result.mode);
      return resp.result.mode;
    } catch (err) {
      privacyLog('[privacy] failed to hydrate privacy mode: %o', err);
      return null;
    }
  }
);

const privacySlice = createSlice({
  name: 'privacy',
  initialState,
  reducers: {
    /** Set the current Privacy Mode (from boot hydration or a settings change). */
    setPrivacyMode: (state, action: PayloadAction<PrivacyMode>) => {
      privacyLog('[privacy] setPrivacyMode %s', action.payload);
      state.privacyMode = action.payload;
    },
    /** Append a disclosure for a thread, capping the ledger length. */
    pushDisclosureForThread: (
      state,
      action: PayloadAction<{ threadId: string; disclosure: PrivacyDisclosure }>
    ) => {
      const { threadId, disclosure } = action.payload;
      const list = (state.disclosuresByThread[threadId] ??= []);
      list.push(disclosure);
      if (list.length > MAX_DISCLOSURES_PER_THREAD) {
        list.splice(0, list.length - MAX_DISCLOSURES_PER_THREAD);
      }
      // Mark the thread as having a live external transfer so the status pill
      // flips off-device. This is independent of the dismissible ledger above:
      // dismissing the card does NOT clear it (the transfer is still active) —
      // only the turn-boundary `clearActiveExternalForThread` does.
      if (disclosure.isExternal) {
        state.activeExternalByThread[threadId] = true;
      }
      privacyLog(
        '[privacy] pushDisclosureForThread thread=%s service=%s external=%s depth=%d',
        threadId,
        disclosure.service,
        String(disclosure.isExternal),
        list.length
      );
    },
    /** Dismiss a single disclosure by id (the only user action in S3). */
    dismissDisclosureForThread: (
      state,
      action: PayloadAction<{ threadId: string; id: string }>
    ) => {
      const { threadId, id } = action.payload;
      const list = state.disclosuresByThread[threadId];
      if (!list) return;
      const next = list.filter(d => d.id !== id);
      if (next.length === 0) {
        delete state.disclosuresByThread[threadId];
      } else {
        state.disclosuresByThread[threadId] = next;
      }
      privacyLog('[privacy] dismissDisclosureForThread thread=%s id=%s', threadId, id);
    },
    /** Clear all disclosures for a thread (e.g. on turn start / thread reset). */
    clearDisclosuresForThread: (state, action: PayloadAction<{ threadId: string }>) => {
      delete state.disclosuresByThread[action.payload.threadId];
      privacyLog('[privacy] clearDisclosuresForThread thread=%s', action.payload.threadId);
    },
    /**
     * Clear the live external-transfer flag for a thread. Dispatched by
     * ChatRuntimeProvider on the turn boundary (chat_done / chat_error /
     * disconnect reconcile) so the pill returns to on-device once the turn's
     * external activity is over — even though the (un-dismissed) ledger entries
     * remain for the in-chat card's history.
     */
    clearActiveExternalForThread: (state, action: PayloadAction<{ threadId: string }>) => {
      if (state.activeExternalByThread[action.payload.threadId]) {
        delete state.activeExternalByThread[action.payload.threadId];
        privacyLog('[privacy] clearActiveExternalForThread thread=%s', action.payload.threadId);
      }
    },
  },
  extraReducers: builder => {
    // On identity flip / sign-out, drop per-user disclosure history. The
    // privacy mode is a core-side setting re-hydrated on the next boot, so it
    // is safe to reset here too.
    builder.addCase(resetUserScopedState, () => initialState);
    builder.addCase(hydratePrivacyMode.fulfilled, (state, action) => {
      if (action.payload) state.privacyMode = action.payload;
    });
  },
});

export const {
  setPrivacyMode,
  pushDisclosureForThread,
  dismissDisclosureForThread,
  clearDisclosuresForThread,
  clearActiveExternalForThread,
} = privacySlice.actions;

export default privacySlice.reducer;
