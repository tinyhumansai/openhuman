/**
 * The runtime reads must degrade, not throw, when no runtime is mounted — and
 * must report the ADAPTER's real capabilities when one is.
 *
 * The capability half is the important one. `useOpenHumanExternalStore`
 * implements `onNew` / `onCancel` and neither `onEdit` nor
 * `setMessages`, so assistant-ui reports `edit` and `switchToBranch` as false.
 * The transcript renders no edit composer and no `BranchPickerPrimitive`
 * because of that, and this test is what would fail the day someone wires an
 * affordance to a capability the adapter cannot honour.
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { renderHook } from '@testing-library/react';
import type { ReactNode } from 'react';
import { Provider } from 'react-redux';
import { describe, expect, it } from 'vitest';

import { AssistantUiRuntimeProvider } from '../../../../providers/AssistantUiRuntimeProvider';
import chatRuntimeReducer from '../../../../store/chatRuntimeSlice';
import threadReducer from '../../../../store/threadSlice';
import { useAuiEditCapabilities, useAuiThreadRunning } from './auiThreadState';

function withRuntime(threadId: string | null) {
  const store = configureStore({
    reducer: combineReducers({ thread: threadReducer, chatRuntime: chatRuntimeReducer }),
  });
  return ({ children }: { children: ReactNode }) => (
    <Provider store={store}>
      <AssistantUiRuntimeProvider threadId={threadId}>{children}</AssistantUiRuntimeProvider>
    </Provider>
  );
}

describe('auiThreadState', () => {
  it('reports undefined running state with no runtime mounted', () => {
    const { result } = renderHook(() => useAuiThreadRunning());
    expect(result.current).toBeUndefined();
  });

  it('reports no edit or branch capability with no runtime mounted', () => {
    const { result } = renderHook(() => useAuiEditCapabilities());
    expect(result.current).toEqual({ canEdit: false, canSwitchToBranch: false });
  });

  it('reports a concrete running state once a runtime is mounted', () => {
    const { result } = renderHook(() => useAuiThreadRunning(), { wrapper: withRuntime('t-caps') });
    expect(result.current).toBe(false);
  });

  it('reports the external-store adapter as supporting neither edit nor branching', () => {
    const { result } = renderHook(() => useAuiEditCapabilities(), {
      wrapper: withRuntime('t-caps'),
    });
    expect(result.current).toEqual({ canEdit: false, canSwitchToBranch: false });
  });
});
