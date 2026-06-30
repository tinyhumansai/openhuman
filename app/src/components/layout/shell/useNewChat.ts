import { useCallback } from 'react';
import { useNavigate } from 'react-router-dom';

import {
  GENERAL_TAB_VALUE,
  isThreadVisibleInTab,
} from '../../../pages/conversations/utils/threadFilter';
import { setActiveAccount } from '../../../store/accountsSlice';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import { createNewThread, loadThreadMessages, setSelectedThread } from '../../../store/threadSlice';
import { AGENT_ACCOUNT_ID } from '../../../utils/accountsFullscreen';
import { chatThreadPath } from '../../../utils/chatRoutes';

/**
 * The "New Chat" action behind the ⌘N / Ctrl+N shortcut.
 *
 * Unlike {@link useHomeNav}, this always lands on a *blank* thread regardless of
 * the current route. `useHomeNav`'s off-chat branch only navigates to `/chat`
 * and lets the mounting Conversations page own blank-thread landing — but that
 * restores the persisted `selectedThreadId` first, so from a non-chat route it
 * would reopen the previous conversation instead of starting a new one. Here we
 * explicitly select/create the blank thread before navigating, so a new chat is
 * always a new chat:
 *
 *  - switch back to the agent account (a selected connected app would otherwise
 *    keep rendering its webview instead of the agent thread);
 *  - reuse an existing empty thread if one exists (avoids piling up blanks),
 *    else create one;
 *  - select + load it and navigate straight to it. Selecting the thread before
 *    navigation also prevents the Conversations page from racing to create a
 *    second blank thread on mount.
 *
 * A thread counts as empty (reusable) only when it has no server message
 * count, no locally-cached messages, and no in-flight assistant turn. The first
 * two guard the post-send window where `addMessageLocal` has populated
 * `messagesByThreadId` (or the thread-list `messageCount`) but the other lags;
 * the `pendingSendThreadIds` marker (set synchronously the instant the user
 * sends, before `addMessageLocal` is even awaited) closes the earliest
 * optimistic-send window, and the streaming buffer covers later ones. We key
 * off "is this thread actually occupied?" rather than "is it selected?" so a
 * genuinely-blank current chat is still reused (no piling up of empties) while
 * a chat with a send in flight is never reopened.
 *
 * Only **General**-tab threads are reuse candidates (same
 * `isThreadVisibleInTab(..., GENERAL_TAB_VALUE)` filter the `/chat` landing
 * uses), so New Chat never lands on a hidden task/subconscious/parented
 * session.
 */
export function useNewChat(): () => void {
  const navigate = useNavigate();
  const dispatch = useAppDispatch();
  const threads = useAppSelector(state => state.thread.threads);
  const messagesByThreadId = useAppSelector(state => state.thread.messagesByThreadId);
  // Optional-chained so minimal test stores without the chatRuntime reducer
  // still resolve (mirrors `selectPanelLayout`'s defensive access).
  const streamingByThread = useAppSelector(state => state.chatRuntime?.streamingAssistantByThread);
  const pendingSendThreadIds = useAppSelector(state => state.chatRuntime?.pendingSendThreadIds);

  return useCallback(() => {
    dispatch(setActiveAccount(AGENT_ACCOUNT_ID));

    const empty = threads.find(
      thr =>
        isThreadVisibleInTab(thr, GENERAL_TAB_VALUE) &&
        (thr.messageCount ?? 0) === 0 &&
        (messagesByThreadId[thr.id]?.length ?? 0) === 0 &&
        !streamingByThread?.[thr.id] &&
        !pendingSendThreadIds?.[thr.id]
    );
    if (empty) {
      dispatch(setSelectedThread(empty.id));
      void dispatch(loadThreadMessages(empty.id));
      navigate(chatThreadPath(empty.id));
      return;
    }

    void dispatch(createNewThread())
      .unwrap()
      .then(thr => {
        dispatch(setSelectedThread(thr.id));
        void dispatch(loadThreadMessages(thr.id));
        navigate(chatThreadPath(thr.id));
      })
      .catch(err => {
        // Don't silently drop the primary New Chat path — log so the failure is
        // diagnosable. The user stays where they are (no broken navigation).
        console.error('[new-chat] createNewThread failed', err);
      });
  }, [navigate, dispatch, threads, messagesByThreadId, streamingByThread, pendingSendThreadIds]);
}
