import { useMemo } from 'react';

import { ConversationsPage } from '../features/conversations/Conversations';
import {
  ChatMascotOverlay,
  ChatMascotProvider,
  ChatMascotStage,
  MASCOT_TRANSITION_MS,
  prefersReducedMotion,
} from '../features/human/chatMascot';
import { useAppSelector } from '../store/hooks';
import { selectChatMascotDismissed, selectChatMascotExpanded } from '../store/mascotSlice';

/**
 * Width of the mascot's voice stage. Used twice on purpose: the outer column
 * animates between `0` and this width (which is what makes the transcript
 * reflow), while the inner panel keeps it fixed and right-anchored so the stage
 * slides in from the edge instead of being squashed open.
 */
const STAGE_WIDTH = 'min(38vw, 520px)';

/** Shared with the mascot's own travel so the column and the mascot land together. */
const STAGE_TRANSITION = `width ${MASCOT_TRANSITION_MS}ms cubic-bezier(0.2, 0.7, 0.2, 1)`;

/**
 * The unified chat surface (`/chat`).
 *
 * Merges what used to be two tabs. The mascot lives here full-time: docked as a
 * small figure standing on the composer, or — one click later — scaled up into
 * the right-hand voice stage that replaced the standalone Human page. The
 * transcript and the text composer stay live in both states, so voice and text
 * are the same conversation rather than two places to have it.
 *
 * [ui-flow] chat: docked mascot ⇄ voice stage (right column, animated width)
 */
const Accounts = () => {
  const mascotDismissed = useAppSelector(selectChatMascotDismissed);
  // Dismissing hides the dock, which would leave the overlay parked off-screen
  // at opacity 0 — an invisible Rive canvas still re-rendering on every lipsync
  // frame, and a poll hunting for an anchor that will never mount. Unmount it.
  const mascotExpanded = useAppSelector(selectChatMascotExpanded) && !mascotDismissed;
  // Read per render rather than once: the OS setting can change while the app
  // is open, and every toggle re-renders this component anyway.
  const reduceMotion = prefersReducedMotion();

  // Stable element so toggling the stage — and the mascot's ~60fps lipsync
  // re-render if it ever reaches this subtree — never reconciles the (heavy)
  // chat tree. Same guard the Human page needed in #5357; its props are constant.
  const chatPanel = useMemo(() => <ConversationsPage />, []);

  return (
    <div
      // `h-full` makes this page fill the shell's content box edge-to-edge.
      className="relative flex h-full overflow-hidden"
      data-testid="accounts-page"
      data-analytics-id="chat-right-sidebar">
      <ChatMascotProvider>
        <main className="relative flex min-w-0 flex-1 flex-row overflow-hidden">
          <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
            {chatPanel}
          </div>

          <div
            className="relative flex-none overflow-hidden"
            // The transition is inline rather than a class because it shares
            // MASCOT_TRANSITION_MS with the mascot's own travel. That means a
            // `motion-reduce:` class cannot switch it off — an inline
            // declaration wins — so the preference is applied here instead,
            // matching `prefersReducedMotion()` in ChatMascotOverlay. Without
            // this the mascot snaps while the column slides.
            style={{
              width: mascotExpanded ? STAGE_WIDTH : '0px',
              transition: reduceMotion ? undefined : STAGE_TRANSITION,
            }}
            data-testid="chat-mascot-stage-column"
            data-expanded={mascotExpanded ? 'true' : 'false'}>
            {/* Unmounted while docked rather than merely clipped: a
                  zero-width column would otherwise leave the mic button and the
                  speak-replies switch off-screen but still in the tab order,
                  and would keep MicComposer enumerating audio devices for a
                  surface nobody can see. */}
            {mascotExpanded && (
              <div className="absolute inset-y-0 right-0 py-3 pl-3" style={{ width: STAGE_WIDTH }}>
                <ChatMascotStage />
              </div>
            )}
          </div>
        </main>
        {!mascotDismissed && <ChatMascotOverlay />}
      </ChatMascotProvider>
    </div>
  );
};

export default Accounts;
