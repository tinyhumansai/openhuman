/**
 * `useAuiThreadId` — which OpenHuman thread a component *inside* the transcript
 * belongs to.
 *
 * Two live consumers depend on it, both in `ChatToolParts`: the parked
 * `composio_connect` card (which reads `pendingApprovalByThread[threadId]` to
 * find the toolkit to connect) and the sub-agent drawer host. Both would
 * "work" if the hook were replaced by a Redux read of `selectedThreadId` —
 * on the home chat those are the same string, which is exactly why the
 * substitution is easy to make and hard to notice.
 *
 * They are not the same string everywhere. `AssistantUiRuntimeProvider` takes
 * an explicit `threadId` and the Workflow Copilot mounts a second runtime on
 * its own builder thread; on that surface a `selectedThreadId` read paints the
 * home chat's approval into the copilot's transcript, and answers the wrong
 * gate. `AssistantUiRuntimeProvider.threadScope.test.tsx` pins that rule for
 * messages and writes — this pins it for the thread-identity context, which is
 * a separate value with no coverage of its own.
 */
import { configureStore } from '@reduxjs/toolkit';
import { render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { describe, expect, it, vi } from 'vitest';

import chatRuntimeReducer from '../../store/chatRuntimeSlice';
import threadReducer from '../../store/threadSlice';
import { AssistantUiRuntimeProvider, useAuiThreadId } from '../AssistantUiRuntimeProvider';

vi.mock('../../services/api/threadApi', () => ({
  threadApi: {
    getDerivedTranscript: vi
      .fn()
      .mockResolvedValue({
        threadId: null,
        items: [],
        total: 0,
        hasMore: false,
        hasTranscript: false,
      }),
  },
}));

const SELECTED = 'home-chat';
const COPILOT = 'copilot-builder';

function buildStore() {
  const store = configureStore({
    reducer: { thread: threadReducer, chatRuntime: chatRuntimeReducer },
  });
  return store;
}

/** Names the thread it believes it is rendering inside. */
function ThreadIdProbe({ label }: { label: string }) {
  const threadId = useAuiThreadId();
  return <span data-testid={label}>{threadId ?? 'none'}</span>;
}

function renderWith(store: ReturnType<typeof buildStore>, ui: React.ReactNode) {
  return render(<Provider store={store}>{ui}</Provider>);
}

describe('useAuiThreadId', () => {
  it('reports the runtime own thread, not the selected one', () => {
    const store = buildStore();
    store.dispatch({ type: 'thread/setSelectedThread', payload: SELECTED });

    renderWith(
      store,
      <AssistantUiRuntimeProvider threadId={COPILOT}>
        <ThreadIdProbe label="copilot" />
      </AssistantUiRuntimeProvider>
    );

    // A `selectedThreadId` read would say `home-chat` here — and the copilot's
    // connect card would then answer the home chat's parked gate.
    expect(screen.getByTestId('copilot')).toHaveTextContent(COPILOT);
    expect(screen.getByTestId('copilot')).not.toHaveTextContent(SELECTED);
  });

  it('keeps two simultaneously-mounted runtimes apart', () => {
    const store = buildStore();
    store.dispatch({ type: 'thread/setSelectedThread', payload: SELECTED });

    renderWith(
      store,
      <>
        <AssistantUiRuntimeProvider>
          <ThreadIdProbe label="home" />
        </AssistantUiRuntimeProvider>
        <AssistantUiRuntimeProvider threadId={COPILOT}>
          <ThreadIdProbe label="copilot" />
        </AssistantUiRuntimeProvider>
      </>
    );

    // The app-wide mount follows the selection; the copilot's does not.
    expect(screen.getByTestId('home')).toHaveTextContent(SELECTED);
    expect(screen.getByTestId('copilot')).toHaveTextContent(COPILOT);
  });

  it('answers null outside any runtime, rather than guessing the selection', () => {
    const store = buildStore();
    store.dispatch({ type: 'thread/setSelectedThread', payload: SELECTED });

    renderWith(store, <ThreadIdProbe label="orphan" />);

    // A consumer with no runtime above it has no thread — inventing one would
    // let a card act on a thread nobody mounted it for.
    expect(screen.getByTestId('orphan')).toHaveTextContent('none');
  });
});
