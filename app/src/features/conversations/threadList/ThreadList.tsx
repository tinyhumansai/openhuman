import type { RefObject } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import type { Thread } from '../../../types/thread';
import { isImeCompositionKeyEvent } from '../Conversations';

export interface ThreadListProps {
  /** Threads visible after the sidebar's search/tab filtering. */
  threads: Thread[];
  selectedThreadId: string | null;
  /** Free-text thread-title search. */
  search: string;
  onSearchChange: (value: string) => void;
  onCreateThread: () => void;
  /** Select a thread (owns dispatch + message load + route sync). */
  onSelectThread: (threadId: string) => void;
  /** Stable, human-readable title for a thread id. */
  resolveTitle: (threadId: string) => string;
  onRequestDelete: (thread: Thread) => void;
  // Inline title rename — controlled by the parent so the edit state stays
  // co-located with the rest of the panel's thread state.
  editingThreadId: string | null;
  editTitleValue: string;
  editTitleInputRef: RefObject<HTMLInputElement | null>;
  onEditTitleValueChange: (value: string) => void;
  onStartEditTitle: (threadId: string) => void;
  onCommitTitle: (threadId: string) => void;
  onCancelEditTitle: () => void;
  onBlurTitle: (threadId: string) => void;
}

/**
 * The conversations left rail: thread-title search, a "new conversation" row,
 * and the scrollable thread list with inline rename + delete affordances.
 * Extracted verbatim from the panel (Phase 1 shell split) — presentational,
 * driven entirely by props so it can be reused by the page and sidebar shells.
 */
export function ThreadList({
  threads,
  selectedThreadId,
  search,
  onSearchChange,
  onCreateThread,
  onSelectThread,
  resolveTitle,
  onRequestDelete,
  editingThreadId,
  editTitleValue,
  editTitleInputRef,
  onEditTitleValueChange,
  onStartEditTitle,
  onCommitTitle,
  onCancelEditTitle,
  onBlurTitle,
}: ThreadListProps) {
  const { t } = useT();
  return (
    // Card background / rounded corners come from TwoPanelLayout's pane styling.
    <div className="h-full flex flex-col">
      {/* Thread search — flush full-width input, mirrors the settings search. */}
      <div className="relative border-b border-line-subtle">
        <span className="pointer-events-none absolute inset-y-0 left-3 flex items-center text-content-faint">
          <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M21 21l-4.35-4.35M11 19a8 8 0 100-16 8 8 0 000 16z"
            />
          </svg>
        </span>
        <input
          type="text"
          value={search}
          onChange={e => onSearchChange(e.target.value)}
          onKeyDown={e => {
            if (e.key === 'Escape' && search) {
              e.preventDefault();
              onSearchChange('');
            }
          }}
          placeholder={t('chat.searchThreads')}
          aria-label={t('chat.searchThreads')}
          data-testid="chat-thread-search-input"
          className="w-full border-0 bg-transparent py-2.5 pl-10 pr-10 text-sm text-content placeholder:text-stone-400 focus:outline-none focus:ring-0 dark:placeholder:text-neutral-500"
        />
        {search && (
          <button
            type="button"
            onClick={() => onSearchChange('')}
            aria-label={t('settings.settingsSearch.clear')}
            data-testid="chat-thread-search-clear"
            className="absolute inset-y-0 right-2 flex items-center px-1 text-content-faint hover:text-content-secondary">
            <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M6 18L18 6M6 6l12 12"
              />
            </svg>
          </button>
        )}
      </div>
      {/* New conversation — a subtle, centered thread-style row (not a loud
          button), below the search and above the thread list. */}
      <button
        type="button"
        data-testid="new-thread-button"
        data-analytics-id="chat-sidebar-new-thread"
        onClick={onCreateThread}
        title={t('chat.newThreadShortcut')}
        className="group w-full cursor-pointer border-b border-line-subtle/60 opacity-50 px-3 py-2 transition-colors hover:bg-surface-hover dark:border-line/60">
        <div className="flex items-center justify-center gap-1.5">
          <svg
            className="h-3.5 w-3.5 flex-shrink-0 text-content-muted"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
          </svg>
          <span className="truncate text-xs text-content-secondary">
            {t('chat.newConversation')}
          </span>
        </div>
      </button>
      <div className="flex-1 overflow-y-auto">
        {threads.length === 0 ? (
          <p className="px-4 py-6 text-xs text-content-faint text-center">{t('chat.noThreads')}</p>
        ) : (
          threads.map(thread => (
            <div
              key={thread.id}
              data-testid={`thread-row-${thread.id}`}
              data-analytics-id="chat-sidebar-thread-row"
              role="button"
              tabIndex={0}
              onClick={() => onSelectThread(thread.id)}
              onKeyDown={e => {
                if (e.target !== e.currentTarget) return;
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  onSelectThread(thread.id);
                }
              }}
              className={`w-full text-left px-3 py-1.5 border-b border-line-subtle/60 dark:border-line/60 transition-colors group cursor-pointer ${
                selectedThreadId === thread.id
                  ? 'bg-primary-50 dark:bg-primary-900/30 border-l-2 border-l-primary-500'
                  : 'hover:bg-surface-hover'
              }`}>
              <div className="flex items-center justify-between">
                {editingThreadId === thread.id ? (
                  <input
                    ref={editTitleInputRef}
                    value={editTitleValue}
                    onClick={e => e.stopPropagation()}
                    onChange={e => onEditTitleValueChange(e.target.value)}
                    onKeyDown={e => {
                      e.stopPropagation();
                      // Ignore the Enter that confirms an IME composition
                      // candidate (CJK input) so it doesn't prematurely commit.
                      if (isImeCompositionKeyEvent(e)) return;
                      if (e.key === 'Enter') {
                        e.preventDefault();
                        onCommitTitle(thread.id);
                      } else if (e.key === 'Escape') {
                        // Escape is an explicit cancel — suppress the commit the
                        // ensuing blur would otherwise fire.
                        onCancelEditTitle();
                      }
                    }}
                    onBlur={() => onBlurTitle(thread.id)}
                    aria-label={t('chat.editThreadTitle')}
                    data-testid={`thread-title-input-${thread.id}`}
                    className="h-5 min-w-0 flex-1 border-b border-primary-400 bg-transparent py-0 text-xs font-medium leading-none text-content-secondary outline-none"
                    autoFocus
                  />
                ) : (
                  <p
                    className={`text-xs truncate flex-1 ${
                      selectedThreadId === thread.id
                        ? 'font-medium text-primary-700 dark:text-primary-200'
                        : 'text-content-secondary'
                    }`}>
                    {resolveTitle(thread.id)}
                  </p>
                )}
                <button
                  type="button"
                  data-analytics-id="chat-sidebar-edit-thread-title"
                  onClick={e => {
                    e.stopPropagation();
                    onStartEditTitle(thread.id);
                  }}
                  aria-label={t('chat.editThreadTitle')}
                  title={t('chat.editThreadTitle')}
                  className="ml-2 p-1 rounded opacity-0 group-hover:opacity-100 hover:bg-surface-strong dark:bg-surface-muted dark:hover:bg-surface-muted text-content-faint hover:text-primary-500 transition-all flex-shrink-0">
                  <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M15.232 5.232l3.536 3.536m-2.036-5.036a2.5 2.5 0 113.536 3.536L6.5 21.036H3v-3.572L16.732 3.732z"
                    />
                  </svg>
                </button>
                <button
                  type="button"
                  data-analytics-id="chat-sidebar-delete-thread"
                  onClick={e => {
                    e.stopPropagation();
                    onRequestDelete(thread);
                  }}
                  className="ml-2 p-1 rounded opacity-0 group-hover:opacity-100 hover:bg-surface-strong dark:bg-surface-muted dark:hover:bg-surface-muted text-content-faint hover:text-coral-500 transition-all flex-shrink-0"
                  title={t('chat.deleteThread')}>
                  <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M6 18L18 6M6 6l12 12"
                    />
                  </svg>
                </button>
              </div>
            </div>
          ))
        )}
      </div>
    </div>
  );
}
