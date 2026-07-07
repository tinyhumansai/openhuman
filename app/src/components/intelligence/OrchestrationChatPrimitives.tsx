/**
 * Presentational primitives for the TinyPlace Orchestration tab's chat surface:
 * a sidebar list row ({@link ChatListButton}) and a message bubble
 * ({@link MessageBubble}). Extracted from the tab container so both the sidebar
 * and focus pane render chats identically.
 */
import type { ReactElement } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { ChatMessage, ChatWindow } from '../../lib/orchestration/useOrchestrationChats';
import { formatTime } from './orchestrationTabHelpers';

export interface ChatListButtonProps {
  chat: ChatWindow;
  selected: boolean;
  onSelect: () => void;
  contactBadge?: string | null;
}

export function ChatListButton({
  chat,
  selected,
  onSelect,
  contactBadge,
}: ChatListButtonProps): ReactElement {
  const { t } = useT();
  return (
    <button
      type="button"
      data-testid={`tinyplace-chat-${chat.id}`}
      onClick={onSelect}
      className={`flex w-full items-start gap-3 border-b border-line-subtle px-3 py-3 text-left transition last:border-b-0 hover:bg-surface-hover ${
        selected ? 'bg-surface-muted' : ''
      }`}>
      <span className="mt-0.5 flex h-9 w-9 flex-none items-center justify-center rounded-lg border border-line bg-surface-strong text-xs font-semibold text-content-secondary">
        {chat.kind === 'subconscious' ? 'S' : chat.kind === 'master' ? 'M' : '#'}
      </span>
      <span className="min-w-0 flex-1">
        <span className="flex items-center justify-between gap-2">
          <span className="truncate text-sm font-semibold text-content">{chat.title}</span>
          <span className="flex-none text-[10px] text-content-faint">
            {formatTime(chat.lastTimestamp)}
          </span>
        </span>
        <span className="mt-0.5 block truncate text-[11px] text-content-muted">
          {chat.kind === 'subconscious'
            ? t('tinyplaceOrchestration.subconsciousBadge')
            : chat.subtitle}
        </span>
        <span className="mt-1 flex items-center gap-2">
          <span className="min-w-0 flex-1 truncate text-xs text-content-faint">{chat.preview}</span>
          {chat.unread > 0 ? (
            <span className="flex-none rounded-full bg-ocean-500 px-1.5 py-0.5 text-[10px] font-semibold text-content-inverted">
              {chat.unread}
            </span>
          ) : null}
          {!chat.pinned ? (
            <span
              className={`flex-none rounded-full px-1.5 py-0.5 text-[10px] font-medium ${
                chat.active
                  ? 'bg-sage-100 text-sage-700 dark:bg-sage-500/15 dark:text-sage-300'
                  : 'bg-surface-strong text-content-faint'
              }`}>
              {chat.active
                ? t('tinyplaceOrchestration.active')
                : t('tinyplaceOrchestration.inactive')}
            </span>
          ) : null}
          {contactBadge ? (
            <span className="flex-none rounded-full bg-surface-strong px-1.5 py-0.5 text-[10px] font-medium text-content-faint">
              {t(contactBadge)}
            </span>
          ) : null}
        </span>
      </span>
    </button>
  );
}

export function MessageBubble({ message }: { message: ChatMessage }): ReactElement {
  return (
    <div className="flex gap-2">
      <div className="mt-1.5 h-2 w-2 flex-none rounded-full bg-ocean-500" />
      <div className="min-w-0 rounded-lg border border-line bg-surface px-3 py-2 shadow-soft">
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
          <span className="text-xs font-semibold text-content-secondary">{message.from}</span>
          <span className="text-[10px] text-content-faint">{formatTime(message.timestamp)}</span>
        </div>
        <p
          className={`mt-1 whitespace-pre-wrap break-words text-sm ${
            message.encrypted ? 'text-content-muted' : 'text-content'
          }`}>
          {message.body}
        </p>
      </div>
    </div>
  );
}
