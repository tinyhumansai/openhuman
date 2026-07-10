/**
 * Redux slice for user-actionable runtime errors (#3931).
 *
 * Holds the live set of expected-user-state failures (insufficient BYO credits,
 * managed-budget exhaustion) surfaced in the desktop shell's error panel.
 * In-memory only: entries survive route changes and background-job completion
 * (the panel is mounted once in the shell) but reset on app restart and on
 * user switch. Cross-restart durability is a planned follow-up — see the PR.
 *
 * Dedupe: entries are keyed by `descriptor.id` (`kind:scope:provider`). A repeat
 * of the same state bumps `count` + `lastSeenAt` instead of stacking duplicates,
 * so the panel never becomes a noisy raw-error log.
 */
import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

import type { UserActionableError, UserErrorDescriptor } from '../types/userError';
import { resetUserScopedState } from './resetActions';

export interface UserErrorsState {
  byId: Record<string, UserActionableError>;
  /** Insertion order of ids (newest appended). */
  order: string[];
}

const initialState: UserErrorsState = { byId: {}, order: [] };

const userErrorsSlice = createSlice({
  name: 'userErrors',
  initialState,
  reducers: {
    /**
     * Record an occurrence of a classified user-actionable error. New ids are
     * appended; existing active ids increment `count`; a previously-resolved id
     * re-opens. `at` (epoch ms) is passed in so the reducer stays pure.
     */
    reportUserError(state, action: PayloadAction<{ descriptor: UserErrorDescriptor; at: number }>) {
      const { descriptor, at } = action.payload;
      const existing = state.byId[descriptor.id];
      if (!existing) {
        state.byId[descriptor.id] = { ...descriptor, occurredAt: at, lastSeenAt: at, count: 1 };
        state.order.push(descriptor.id);
        return;
      }
      // Refresh presentation fields in case the descriptor copy/action changed,
      // keep the original first-seen timestamp, bump recurrence bookkeeping, and
      // clear any prior resolution (the state is active again).
      state.byId[descriptor.id] = {
        ...existing,
        ...descriptor,
        occurredAt: existing.occurredAt,
        lastSeenAt: at,
        count: existing.count + 1,
        resolvedAt: undefined,
      };
    },

    /** Mark an entry resolved (acted-on). It drops out of the active list. */
    resolveUserError(state, action: PayloadAction<{ id: string; at: number }>) {
      const entry = state.byId[action.payload.id];
      if (entry) entry.resolvedAt = action.payload.at;
    },

    /** Remove an entry entirely (explicit user dismissal). */
    dismissUserError(state, action: PayloadAction<{ id: string }>) {
      if (state.byId[action.payload.id]) {
        delete state.byId[action.payload.id];
        state.order = state.order.filter(id => id !== action.payload.id);
      }
    },

    /** Drop all resolved entries (housekeeping). */
    clearResolvedUserErrors(state) {
      for (const id of [...state.order]) {
        if (state.byId[id]?.resolvedAt !== undefined) {
          delete state.byId[id];
          state.order = state.order.filter(other => other !== id);
        }
      }
    },
  },
  extraReducers: builder => {
    // Privacy: never leak one user's actionable errors into another session.
    builder.addCase(resetUserScopedState, () => initialState);
  },
});

export const { reportUserError, resolveUserError, dismissUserError, clearResolvedUserErrors } =
  userErrorsSlice.actions;

export default userErrorsSlice.reducer;
