import { useCallback } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import { setActiveAccount } from '../../../store/accountsSlice';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import { createNewThread, loadThreadMessages, setSelectedThread } from '../../../store/threadSlice';
import { AGENT_ACCOUNT_ID } from '../../../utils/accountsFullscreen';
import { chatThreadPath } from '../../../utils/chatRoutes';

/**
 * The shell's "Home" action — shared by the sidebar header (expanded) and the
 * collapsed icon rail so both behave identically.
 *
 * Home → the unified chat on a blank thread. When we're NOT already on chat,
 * just navigate and let the mounting Conversations page own blank-thread landing
 * (avoids a duplicate-create race). When already on chat (no remount), reset to
 * a blank thread here: reuse an existing empty one, else create one.
 */
export function useHomeNav(): () => void {
  const navigate = useNavigate();
  const location = useLocation();
  const dispatch = useAppDispatch();
  const threads = useAppSelector(state => state.thread.threads);

  return useCallback(() => {
    // Switch back to the agent account first — otherwise a selected connected
    // app (WhatsApp/Slack/…) keeps Accounts rendering its webview instead of the
    // blank agent thread.
    dispatch(setActiveAccount(AGENT_ACCOUNT_ID));
    const onChat = location.pathname === '/chat' || location.pathname.startsWith('/chat/');
    if (!onChat) {
      navigate('/chat');
      return;
    }
    const empty = threads.find(thr => (thr.messageCount ?? 0) === 0);
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
      .catch(() => {});
  }, [navigate, location.pathname, dispatch, threads]);
}
