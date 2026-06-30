/**
 * Selectors for the user-actionable runtime errors slice (#3931).
 *
 * Kept in a separate file from the slice (matching `socketSelectors` /
 * `connectivitySelectors`) so the slice never imports the root store, avoiding
 * a slice ↔ store dependency cycle.
 */
import { createSelector } from '@reduxjs/toolkit';

import type { UserActionableError } from '../types/userError';
import type { RootState } from './index';
import type { UserErrorsState } from './userErrorsSlice';

const selectSlice = (state: RootState): UserErrorsState => state.userErrors;

/** All entries in insertion order (including resolved). */
export const selectAllUserErrors = createSelector(selectSlice, slice =>
  slice.order.map(id => slice.byId[id]).filter((e): e is UserActionableError => Boolean(e))
);

/** Active (unresolved) entries — what the panel and badge show. */
export const selectActiveUserErrors = createSelector(selectAllUserErrors, all =>
  all.filter(e => e.resolvedAt === undefined)
);

/** Count of active entries — drives the notification badge. */
export const selectActiveUserErrorCount = createSelector(
  selectActiveUserErrors,
  active => active.length
);
