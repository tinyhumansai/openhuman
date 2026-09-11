import debug from 'debug';
import { useState } from 'react';

import { ConfirmDialog } from '../../../components/ui';
import { useT } from '../../../lib/i18n/I18nContext';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import {
  selectChatMascotDismissed,
  selectChatMascotExpanded,
  setChatMascotDismissed,
} from '../../../store/mascotSlice';
import { useChatMascot } from './ChatMascotContext';
import { DOCK_PX } from './geometry';

const dockLog = debug('human:chat-mascot');

/**
 * The small mascot standing on the top-right corner of the composer's input box.
 *
 * Draws **nothing** itself: `ChatMascotOverlay` paints the one shared mascot
 * over this slot. This component only supplies the anchor rect, the hit area,
 * and the accessible label — which is why it is safe for it to sit inside the
 * composer without dragging Rive's per-frame re-render into the chat tree.
 *
 * [ui-flow] chat-mascot: dock click → expanded (stage) → mascot/collapse → docked
 * [ui-flow] chat-mascot: dismiss (×) → confirm → hidden until re-enabled in settings
 */
const ChatMascotDock = () => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const { setDockNode, expand } = useChatMascot();
  const expanded = useAppSelector(selectChatMascotExpanded);
  const dismissed = useAppSelector(selectChatMascotDismissed);
  const [confirmingDismiss, setConfirmingDismiss] = useState(false);

  // Nothing is painted here once the mascot has flown to the stage, so keeping
  // the slot mounted would leave an invisible 64px click target sitting on the
  // composer. Collapsing is the stage's job.
  if (expanded || dismissed) return null;

  return (
    <>
      {/* `group` wraps both controls so the × reveals on hovering the mascot,
          not only on hovering the 16px × itself — which would be near-impossible
          to find, since the dock paints nothing of its own to aim at. */}
      <div
        className="group absolute bottom-full right-2 z-10 -mb-1"
        style={{ width: DOCK_PX, height: DOCK_PX }}>
        <button
          ref={setDockNode}
          type="button"
          // `bottom-full` puts the slot's base exactly on the input box's top
          // edge; the negative margin above drops it a few px so the mascot
          // reads as standing *on* the box rather than floating above it.
          className="h-full w-full rounded-full transition-transform hover:scale-105 focus:outline-hidden focus-visible:ring-2 focus-visible:ring-primary-500 motion-reduce:transition-none"
          aria-label={t('chat.mascot.expand')}
          title={t('chat.mascot.expand')}
          aria-expanded={false}
          data-testid="chat-mascot-dock"
          data-analytics-id="chat-mascot-toggle"
          onClick={expand}
        />

        {/* Hidden until hover/focus so the mascot stays a mascot rather than a
            widget with chrome. `focus-within` keeps it reachable by keyboard,
            where there is no hover to trigger. */}
        <button
          type="button"
          className="absolute -right-1 -top-1 flex h-4 w-4 items-center justify-center rounded-full border border-line bg-surface text-content-faint opacity-0 shadow-soft transition-opacity hover:text-content focus:outline-hidden focus-visible:ring-2 focus-visible:ring-primary-500 group-focus-within:opacity-100 group-hover:opacity-100 motion-reduce:transition-none"
          aria-label={t('chat.mascot.dismiss')}
          title={t('chat.mascot.dismiss')}
          data-testid="chat-mascot-dismiss"
          data-analytics-id="chat-mascot-dismiss"
          onClick={() => {
            dockLog('[chat-mascot][dismiss] prompt opened');
            setConfirmingDismiss(true);
          }}>
          <svg
            className="h-2.5 w-2.5"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
            aria-hidden="true">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={3}
              d="M6 6l12 12M18 6L6 18"
            />
          </svg>
        </button>
      </div>

      {confirmingDismiss && (
        <ConfirmDialog
          titleId="chat-mascot-dismiss-title"
          title={t('chat.mascot.dismissTitle')}
          body={t('chat.mascot.dismissBody')}
          confirmLabel={t('chat.mascot.dismissConfirm')}
          cancelLabel={t('chat.mascot.dismissCancel')}
          onConfirm={() => {
            dockLog('[chat-mascot][dismiss] confirmed — hiding until re-enabled in settings');
            setConfirmingDismiss(false);
            dispatch(setChatMascotDismissed(true));
          }}
          onCancel={() => {
            dockLog('[chat-mascot][dismiss] cancelled — mascot kept');
            setConfirmingDismiss(false);
          }}
        />
      )}
    </>
  );
};

export default ChatMascotDock;
