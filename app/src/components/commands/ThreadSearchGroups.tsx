import { Command } from 'cmdk';
import debug from 'debug';
import { MessageSquare, MessagesSquare } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';

import { threadMatchesQuery } from '../../features/conversations/threadList/groupThreads';
import { useT } from '../../lib/i18n/I18nContext';
import { threadApi } from '../../services/api/threadApi';
import type { Thread, ThreadSearchHit } from '../../types/thread';

const log = debug('openhuman:commands:thread-search');

/** Conversations matched by title, most recent first. */
const MAX_TITLE_MATCHES = 8;
/** Message hits asked of the core per query. */
const MESSAGE_LIMIT = 20;
/** Shortest query sent to the message index (its trigrams need three). */
const MIN_MESSAGE_QUERY = 2;
/** Typing pause before the message index is queried. */
const DEBOUNCE_MS = 200;

const GROUP_CLASS =
  '**:[[cmdk-group-heading]]:px-4 **:[[cmdk-group-heading]]:py-1 **:[[cmdk-group-heading]]:text-xs **:[[cmdk-group-heading]]:uppercase **:[[cmdk-group-heading]]:text-cmd-foreground-muted';
const ITEM_CLASS =
  'flex items-center gap-3 px-4 py-2 cursor-pointer aria-selected:bg-cmd-surface-elevated';

interface Props {
  /** The palette's current input. */
  query: string;
  threads: Thread[];
  onOpenThread: (threadId: string) => void;
}

/**
 * The palette's conversation search: threads whose title matches, then
 * messages anywhere in any thread (`openhuman.threads_search`). Each item lists
 * the query among its keywords so cmdk's own filter keeps what the core found
 * by trigram rather than by exact substring.
 */
export default function ThreadSearchGroups({ query, threads, onOpenThread }: Props) {
  const { t } = useT();
  const needle = query.trim();
  // The last answer and the query it answers; a newer query hides it until
  // its own answer lands, so a stale hit never sits under a fresh query.
  const [results, setResults] = useState<{ query: string; hits: ThreadSearchHit[] }>({
    query: '',
    hits: [],
  });
  const wantsMessages = needle.length >= MIN_MESSAGE_QUERY;
  const hits = wantsMessages && results.query === needle ? results.hits : [];
  const searching = wantsMessages && results.query !== needle;

  useEffect(() => {
    if (!wantsMessages) return;
    let cancelled = false;
    const timer = window.setTimeout(() => {
      threadApi
        .searchMessages(needle, MESSAGE_LIMIT)
        .then(found => {
          if (cancelled) return;
          log('query_chars=%d hits=%d', needle.length, found.length);
          setResults({ query: needle, hits: found });
        })
        .catch(error => {
          if (cancelled) return;
          log('search failed: %o', error);
          setResults({ query: needle, hits: [] });
        });
    }, DEBOUNCE_MS);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [needle, wantsMessages]);

  const titleMatches = useMemo(() => {
    if (!needle) return [];
    return threads
      .filter(thread => threadMatchesQuery(thread.title ?? '', needle))
      .sort((a, b) => new Date(b.lastMessageAt).getTime() - new Date(a.lastMessageAt).getTime())
      .slice(0, MAX_TITLE_MATCHES);
  }, [threads, needle]);

  const titleOf = useMemo(() => {
    const byId = new Map(threads.map(thread => [thread.id, thread.title]));
    return (threadId: string) =>
      byId.get(threadId)?.trim() || t('commandPalette.untitledConversation');
  }, [threads, t]);

  if (!needle) return null;

  return (
    <>
      {titleMatches.length > 0 && (
        <Command.Group heading={t('commandPalette.group.conversations')} className={GROUP_CLASS}>
          {titleMatches.map(thread => (
            <Command.Item
              key={thread.id}
              value={`thread:${thread.id}`}
              keywords={[needle, thread.title ?? '']}
              onSelect={() => onOpenThread(thread.id)}
              data-testid={`palette-thread-${thread.id}`}
              className={ITEM_CLASS}>
              <MessagesSquare className="w-4 h-4 text-cmd-foreground-muted" />
              <span className="flex-1 truncate">{titleOf(thread.id)}</span>
            </Command.Item>
          ))}
        </Command.Group>
      )}
      {hits.length > 0 && (
        <Command.Group heading={t('commandPalette.group.messages')} className={GROUP_CLASS}>
          {hits.map(hit => (
            <Command.Item
              key={`${hit.threadId}:${hit.messageId}`}
              value={`message:${hit.threadId}:${hit.messageId}`}
              keywords={[needle, hit.snippet]}
              onSelect={() => onOpenThread(hit.threadId)}
              data-testid={`palette-message-${hit.messageId}`}
              className={ITEM_CLASS}>
              <MessageSquare className="w-4 h-4 shrink-0 text-cmd-foreground-muted" />
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="truncate">{titleOf(hit.threadId)}</span>
                <span className="truncate text-xs text-cmd-foreground-muted">{hit.snippet}</span>
              </span>
            </Command.Item>
          ))}
        </Command.Group>
      )}
      {searching && hits.length === 0 && (
        <Command.Loading>
          <div className="px-4 py-2 text-xs text-cmd-foreground-muted">
            {t('commandPalette.searchingMessages')}
          </div>
        </Command.Loading>
      )}
    </>
  );
}
