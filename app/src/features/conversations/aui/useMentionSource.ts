/**
 * The composer's `@` mention source: OpenHuman glue that feeds assistant-ui's
 * `unstable_useMentionAdapter` for the vendored `ComposerTriggerPopover`.
 *
 * Two categories:
 * - **Memory** — a search of stored memory (`openhuman.memory_fetch` via
 *   {@link memoryFetch}, the engine's default mode) for the `@query` being
 *   typed, debounced. With memory off the call fails and the category is empty. The adapter's own search is a substring
 *   filter, which would drop semantic hits whose label does not literally
 *   contain the query, so `search` is extended to always include them.
 * - **Files** — the thread's ready artifacts, read from
 *   `chatRuntime.artifactsByThread` (the list the header `ChatFilesChip`
 *   hydrates). No new RPC.
 *
 * Choosing an item inserts `:type[label]{name=id}` through the default
 * directive formatter — the syntax `DirectiveText` renders as a chip in the
 * sent message.
 */
import type { Unstable_TriggerAdapter } from '@assistant-ui/core';
import {
  type Unstable_IconComponent,
  type Unstable_Mention,
  unstable_useMentionAdapter,
  useAuiState,
} from '@assistant-ui/react';
import debug from 'debug';
import { AtSignIcon, BrainIcon, FileIcon } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { type Hit, memoryFetch } from '../../../services/api/memoryApi';
import type { ArtifactSnapshot } from '../../../store/chatRuntimeSlice';
import { useAppSelector } from '../../../store/hooks';

const log = debug('openhuman:chat:mentions');

const FETCH_LIMIT = 8;
const RECALL_DEBOUNCE_MS = 200;
const MIN_QUERY_LENGTH = 2;
const MAX_LABEL_LENGTH = 48;

const ICON_MAP: Record<string, Unstable_IconComponent> = { memory: BrainIcon, files: FileIcon };

const NO_ARTIFACTS: readonly ArtifactSnapshot[] = [];

/** The query of a trailing `@mention` (caret assumed at the end), else `null`. */
export function trailingMentionQuery(text: string): string | null {
  const match = /(?:^|\s)@([^\s@]*)$/.exec(text);
  return match ? (match[1] ?? '') : null;
}

/** One line, no directive delimiters, bounded — so it round-trips as a chip label. */
function directiveSafe(text: string): string {
  const flat = text
    .replace(/[[\]{}]/g, '')
    .replace(/\s+/g, ' ')
    .trim();
  return flat.length > MAX_LABEL_LENGTH
    ? `${flat.slice(0, MAX_LABEL_LENGTH - 1).trimEnd()}…`
    : flat;
}

export function memoryMentionsFromHits(hits: readonly Hit[]): Unstable_Mention[] {
  return hits.map(hit => ({
    id: directiveSafe(hit.id),
    type: 'memory',
    label: directiveSafe(hit.text || hit.meta?.file_path || hit.meta?.thread_id || hit.id),
    description: hit.kind,
    icon: 'memory',
  }));
}

export function fileMentionsFromArtifacts(
  artifacts: readonly ArtifactSnapshot[]
): Unstable_Mention[] {
  return artifacts
    .filter(artifact => artifact.status === 'ready')
    .map(artifact => ({
      id: directiveSafe(artifact.artifactId),
      type: 'file',
      label: directiveSafe(artifact.title),
      ...(artifact.path ? { description: artifact.path } : {}),
      icon: 'files',
    }));
}

/** Spreadable props for `<ComposerTriggerPopover char="@" … />`. */
export function useMentionSource(threadId: string | null) {
  const { t } = useT();
  const query = useAuiState(state => trailingMentionQuery(state.composer.text));
  const artifacts = useAppSelector(state =>
    threadId ? (state.chatRuntime.artifactsByThread[threadId] ?? NO_ARTIFACTS) : NO_ARTIFACTS
  );
  const [memory, setMemory] = useState<Unstable_Mention[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const requestSeq = useRef(0);

  useEffect(() => {
    if (query === null || query.length < MIN_QUERY_LENGTH) return;
    const seq = ++requestSeq.current;
    setIsLoading(true);
    const timer = setTimeout(() => {
      log('fetch: query_len=%d', query.length);
      memoryFetch({ query, limit: FETCH_LIMIT })
        .then(response => memoryMentionsFromHits(response.hits ?? []))
        .catch(error => {
          log('fetch failed, no memory mentions: %o', error);
          return [] as Unstable_Mention[];
        })
        .then(mentions => {
          if (seq !== requestSeq.current) return;
          log('fetch: %d hit(s)', mentions.length);
          setMemory(mentions);
          setIsLoading(false);
        });
    }, RECALL_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [query]);

  const categories = useMemo(
    () => [
      { id: 'memory', label: t('conversations.composer.mention.memory', 'Memory'), items: memory },
      {
        id: 'files',
        label: t('conversations.composer.mention.files', 'Files'),
        items: fileMentionsFromArtifacts(artifacts),
      },
    ],
    [artifacts, memory, t]
  );

  const mention = unstable_useMentionAdapter({
    categories,
    includeModelContextTools: false,
    iconMap: ICON_MAP,
    fallbackIcon: AtSignIcon,
  });

  const adapter = useMemo<Unstable_TriggerAdapter>(() => {
    const base = mention.adapter;
    return {
      categories: () => base.categories(),
      categoryItems: id => base.categoryItems(id),
      search: q => {
        const matched = base.search?.(q) ?? [];
        const seen = new Set(matched.map(item => item.id));
        const semantic = base.categoryItems('memory').filter(item => !seen.has(item.id));
        return [...matched, ...semantic];
      },
    };
  }, [mention.adapter]);

  return {
    ...mention,
    adapter,
    isLoading,
    backLabel: t('conversations.composer.trigger.back', 'Back'),
    loadingLabel: t('conversations.composer.trigger.loading', 'Loading…'),
    emptyCategoriesLabel: t('conversations.composer.trigger.emptyCategories', 'No items available'),
    emptyItemsLabel: t('conversations.composer.mention.empty', 'No matching items'),
  };
}
