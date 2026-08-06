import { useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { ShareCardModal } from './ShareCardModal';

interface ShareMessageButtonProps {
  /** Plain-text agent output to turn into a share card. */
  content: string;
  /** Display name of the agent/profile that produced the output. */
  agentName: string;
  /** Optional thread id, forwarded to inference for log grouping. */
  threadId?: string;
  /** Extra classes for positioning within the message row. */
  className?: string;
}

/**
 * Hover-revealed "Share" affordance rendered under a completed agent message
 * (issue #5006). Opens `ShareCardModal`. Self-contained so the wiring into the
 * large `Conversations.tsx` message list stays a one-liner. Renders nothing when
 * there is no shareable text.
 */
export function ShareMessageButton({
  content,
  agentName,
  threadId,
  className = '',
}: ShareMessageButtonProps) {
  const { t } = useT();
  const [open, setOpen] = useState(false);

  if (!content.trim()) return null;

  return (
    <>
      <button
        type="button"
        data-analytics-id="chat-message-share"
        data-testid="chat-message-share"
        onClick={() => setOpen(true)}
        title={t('share.button')}
        aria-label={t('share.button')}
        className={`p-1 rounded-md opacity-0 group-hover/msg:opacity-100 hover:bg-surface-hover dark:bg-surface-muted dark:hover:bg-surface-muted text-content-faint hover:text-content-secondary transition-all ${className}`}>
        <svg
          className="w-3.5 h-3.5"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
          aria-hidden="true">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M8.7 10.7l6.6-3.4M8.7 13.3l6.6 3.4M18 8a3 3 0 100-6 3 3 0 000 6zM6 15a3 3 0 100-6 3 3 0 000 6zm12 7a3 3 0 100-6 3 3 0 000 6z"
          />
        </svg>
      </button>
      {open ? (
        <ShareCardModal
          content={content}
          agentName={agentName}
          threadId={threadId}
          onClose={() => setOpen(false)}
        />
      ) : null}
    </>
  );
}
