import { createAction } from '@reduxjs/toolkit';

/**
 * Top-level action dispatched on identity flip (user A → user B) and on
 * sign-out. Every user-scoped slice handles this in `extraReducers` and
 * returns its `initialState`. See [#900].
 */
export const resetUserScopedState = createAction('store/resetUserScopedState');
