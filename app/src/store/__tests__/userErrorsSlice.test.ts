import { configureStore } from '@reduxjs/toolkit';
import { describe, expect, it } from 'vitest';

import type { UserErrorDescriptor } from '../../types/userError';
import type { RootState } from '../index';
import { resetUserScopedState } from '../resetActions';
import { selectActiveUserErrorCount, selectActiveUserErrors } from '../userErrorsSelectors';
import reducer, {
  clearResolvedUserErrors,
  dismissUserError,
  reportUserError,
  resolveUserError,
} from '../userErrorsSlice';

const descriptor: UserErrorDescriptor = {
  id: 'budget_exceeded:chat:unknown',
  kind: 'budget_exceeded',
  severity: 'warning',
  scope: 'chat',
  titleKey: 'userErrors.budgetExceeded.title',
  bodyKey: 'userErrors.budgetExceeded.body',
  action: 'open_billing',
};

function makeStore() {
  return configureStore({ reducer: { userErrors: reducer } });
}
const asRoot = (s: unknown) => s as RootState;

describe('userErrorsSlice', () => {
  it('records a new actionable error as active with count 1', () => {
    const store = makeStore();
    store.dispatch(reportUserError({ descriptor, at: 1000 }));
    const active = selectActiveUserErrors(asRoot(store.getState()));
    expect(active).toHaveLength(1);
    expect(active[0]).toMatchObject({
      id: descriptor.id,
      count: 1,
      occurredAt: 1000,
      lastSeenAt: 1000,
    });
    expect(selectActiveUserErrorCount(asRoot(store.getState()))).toBe(1);
  });

  it('dedupes repeats: bumps count + lastSeenAt, keeps occurredAt, no duplicate row', () => {
    const store = makeStore();
    store.dispatch(reportUserError({ descriptor, at: 1000 }));
    store.dispatch(reportUserError({ descriptor, at: 2500 }));
    const active = selectActiveUserErrors(asRoot(store.getState()));
    expect(active).toHaveLength(1);
    expect(active[0]).toMatchObject({ count: 2, occurredAt: 1000, lastSeenAt: 2500 });
  });

  it('resolve drops the entry from the active list; dismiss removes it entirely', () => {
    const store = makeStore();
    store.dispatch(reportUserError({ descriptor, at: 1000 }));

    store.dispatch(resolveUserError({ id: descriptor.id, at: 1500 }));
    expect(selectActiveUserErrors(asRoot(store.getState()))).toHaveLength(0);
    expect(selectActiveUserErrorCount(asRoot(store.getState()))).toBe(0);

    // A resolved entry re-opens if the state recurs.
    store.dispatch(reportUserError({ descriptor, at: 1800 }));
    expect(selectActiveUserErrors(asRoot(store.getState()))).toHaveLength(1);
    expect(selectActiveUserErrors(asRoot(store.getState()))[0].count).toBe(2);

    store.dispatch(dismissUserError({ id: descriptor.id }));
    expect(selectActiveUserErrors(asRoot(store.getState()))).toHaveLength(0);
    expect(store.getState().userErrors.order).not.toContain(descriptor.id);
  });

  it('clearResolvedUserErrors removes only resolved entries', () => {
    const store = makeStore();
    const other: UserErrorDescriptor = {
      ...descriptor,
      id: 'insufficient_credits:chat:unknown',
      kind: 'insufficient_credits',
    };
    store.dispatch(reportUserError({ descriptor, at: 1000 }));
    store.dispatch(reportUserError({ descriptor: other, at: 1000 }));
    store.dispatch(resolveUserError({ id: descriptor.id, at: 1200 }));

    store.dispatch(clearResolvedUserErrors());
    const ids = store.getState().userErrors.order;
    expect(ids).toEqual([other.id]);
  });

  it('clears all entries on user-scoped reset (privacy)', () => {
    const store = makeStore();
    store.dispatch(reportUserError({ descriptor, at: 1000 }));
    store.dispatch(resetUserScopedState());
    expect(selectActiveUserErrors(asRoot(store.getState()))).toHaveLength(0);
    expect(store.getState().userErrors.order).toHaveLength(0);
  });
});
