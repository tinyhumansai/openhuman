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

/**
 * Per-kind presentation for a harness (v2) event row. Differentiates the typed
 * stream (tool_call / tool_result / agent_thinking / approval_request / error)
 * from plain agent/user messages so the orchestration thread reads like a live
 * activity log rather than undifferentiated text. Legacy v1 rows (no
 * `eventKind`) fall through to the default agent-message style.
 */
interface BubbleStyle {
  /** Leading marker: a glyph when the kind has one, else a colored dot. */
  dot: string;
  glyph?: string;
  /** Render the body in a monospace block (tool command / output). */
  mono?: boolean;
  /** Body text tone. */
  tone: string;
  /** Optional left-accent on the bubble. */
  accent: string;
}

function bubbleStyle(kind: ChatMessage['eventKind']): BubbleStyle {
  switch (kind) {
    case 'tool_call':
      return {
        dot: 'text-ocean-500',
        glyph: '▶',
        mono: true,
        tone: 'text-content',
        accent: 'border-l-2 border-l-ocean-400',
      };
    case 'tool_result':
      return {
        dot: 'text-sage-500',
        glyph: '↳',
        mono: true,
        tone: 'text-content-muted',
        accent: 'border-l-2 border-l-sage-400',
      };
    case 'agent_thinking':
      return {
        dot: 'text-content-faint',
        glyph: '∴',
        tone: 'italic text-content-faint',
        accent: '',
      };
    case 'approval_request':
      return {
        dot: 'text-amber-500',
        glyph: '⚠',
        tone: 'text-content',
        accent: 'border-l-2 border-l-amber-400',
      };
    case 'error':
      return {
        dot: 'text-coral-500',
        glyph: '✕',
        tone: 'text-coral-600 dark:text-coral-300',
        accent: 'border-l-2 border-l-coral-400',
      };
    case 'user_prompt':
      return { dot: 'text-sage-500', tone: 'text-content', accent: '' };
    default:
      return { dot: 'text-ocean-500', tone: 'text-content', accent: '' };
  }
}

export function MessageBubble({ message }: { message: ChatMessage }): ReactElement {
  const style = bubbleStyle(message.eventKind);
  return (
    <div className="flex gap-2" data-event-kind={message.eventKind ?? 'v1'}>
      {style.glyph ? (
        <span className={`mt-0.5 flex-none text-xs font-semibold ${style.dot}`}>{style.glyph}</span>
      ) : (
        <div
          className={`mt-1.5 h-2 w-2 flex-none rounded-full ${style.dot.replace('text-', 'bg-')}`}
        />
      )}
      <div
        className={`min-w-0 rounded-lg border border-line bg-surface px-3 py-2 shadow-soft ${style.accent}`}>
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
          <span className="text-xs font-semibold text-content-secondary">{message.from}</span>
          {message.toolName ? (
            <span className="rounded bg-surface-strong px-1.5 py-0.5 font-mono text-[10px] text-content-secondary">
              {message.toolName}
            </span>
          ) : null}
          <span className="text-[10px] text-content-faint">{formatTime(message.timestamp)}</span>
        </div>
        <p
          className={`mt-1 max-h-64 overflow-y-auto whitespace-pre-wrap break-words ${
            style.mono ? 'font-mono text-xs' : 'text-sm'
          } ${message.encrypted ? 'text-content-muted' : style.tone}`}>
          {message.body}
        </p>
      </div>
    </div>
  );
}
