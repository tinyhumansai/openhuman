import {
  ThreadListItem,
  ThreadListNew,
  ThreadListRoot,
} from '@/components/assistant-ui/thread-list';
import {
  AssistantRuntimeProvider,
  ThreadListItemMorePrimitive,
  ThreadListPrimitive,
  type ThreadMessageLike,
  useAuiState,
  useExternalStoreRuntime,
} from '@assistant-ui/react';
import { PinIcon } from 'lucide-react';
import { createContext, useContext, useMemo } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import type { Thread } from '../../../types/thread';
import { folderBasename, groupThreads, isThreadPinned, type ThreadGroupKey } from './groupThreads';

const GROUP_LABEL_KEYS: Record<ThreadGroupKey, string> = {
  pinned: 'chat.sidebar.group.pinned',
  today: 'chat.sidebar.group.today',
  yesterday: 'chat.sidebar.group.yesterday',
  previous7Days: 'chat.sidebar.group.previous7Days',
  previous30Days: 'chat.sidebar.group.previous30Days',
  older: 'chat.sidebar.group.older',
};

interface ThreadListProps {
  threads: Thread[];
  selectedThreadId: string | null;
  onCreateThread: () => void | Promise<void>;
  onSelectThread: (threadId: string) => void;
  resolveTitle: (threadId: string) => string;
  isThreadRunning?: (threadId: string) => boolean;
  unreadThreadIds?: ReadonlySet<string>;
  isPinned?: (thread: Thread) => boolean;
  onTogglePin?: (thread: Thread, pinned: boolean) => void;
  onRequestDelete: (thread: Thread) => void;
  onRenameThread: (threadId: string, title: string) => Promise<void>;
}

const ThreadListHostContext = createContext<ThreadListProps | null>(null);
const EMPTY_MESSAGES: readonly ThreadMessageLike[] = [];

/** Registry row plus OpenHuman's working folder, unread indicator and pin action. */
function ThreadRow() {
  const host = useContext(ThreadListHostContext)!;
  const { t } = useT();
  const id = useAuiState(s => s.threadListItem.id);
  const thread = host.threads.find(item => item.id === id);
  if (!thread) return null;
  const running = Boolean(host.isThreadRunning?.(id));
  const pinned = (host.isPinned ?? isThreadPinned)(thread);
  return (
    <ThreadListItem
      className="flex-none"
      running={running}
      showArchive={false}
      triggerProps={
        {
          'data-testid': `thread-row-${id}`,
          'data-analytics-id': 'chat-sidebar-thread-row',
          // Re-selecting the active thread still synchronizes OpenHuman's route.
          onClick: () => {
            if (host.selectedThreadId === id) host.onSelectThread(id);
          },
          title: thread.actionDir
            ? t('chat.sidebar.workingFolder').replace('{folder}', folderBasename(thread.actionDir))
            : undefined,
        } as React.ComponentPropsWithoutRef<'button'>
      }
      trailing={
        !running && host.unreadThreadIds?.has(id) ? (
          <span
            data-testid={`thread-unread-${id}`}
            role="img"
            aria-label={t('chat.sidebar.unread')}
            className="bg-primary size-1.5 shrink-0 rounded-full"
          />
        ) : null
      }
      menuExtras={
        host.onTogglePin ? (
          <ThreadListItemMorePrimitive.Item
            data-testid={`thread-pin-${id}`}
            onSelect={() => host.onTogglePin?.(thread, !pinned)}
            className="hover:bg-accent focus:bg-accent flex cursor-pointer items-center gap-2 rounded-lg px-2.5 py-1.5 text-sm outline-none">
            <PinIcon aria-hidden className="size-4" />
            {t(pinned ? 'chat.sidebar.unpinThread' : 'chat.sidebar.pinThread')}
          </ThreadListItemMorePrimitive.Item>
        ) : null
      }
    />
  );
}

/** The core owns threads; assistant-ui owns selection and row actions. */
export function ThreadList(props: ThreadListProps) {
  const { threads, resolveTitle } = props;
  const adapterThreads = useMemo(
    () =>
      threads.map(thread => ({
        id: thread.id,
        remoteId: thread.id,
        status: 'regular' as const,
        title: resolveTitle(thread.id),
      })),
    [threads, resolveTitle]
  );
  const runtime = useExternalStoreRuntime({
    messages: EMPTY_MESSAGES,
    convertMessage: message => message,
    isRunning: false,
    onNew: async () => {},
    adapters: {
      threadList: {
        threads: adapterThreads,
        threadId: props.selectedThreadId ?? undefined,
        onSwitchToNewThread: props.onCreateThread,
        onSwitchToThread: props.onSelectThread,
        onRename: props.onRenameThread,
        onDelete: id => {
          const thread = props.threads.find(item => item.id === id);
          if (thread) props.onRequestDelete(thread);
        },
      },
    },
  });
  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <ThreadListHostContext.Provider value={props}>
        <ThreadListView />
      </ThreadListHostContext.Provider>
    </AssistantRuntimeProvider>
  );
}

/** Render only ids already admitted by the runtime, including during async list updates. */
function ThreadListView() {
  const props = useContext(ThreadListHostContext)!;
  const { t } = useT();
  const threadIds = useAuiState(s => s.threads.threadIds);
  const filtered = threadIds.flatMap(id => {
    const thread = props.threads.find(item => item.id === id);
    return thread ? [thread] : [];
  });
  const groups = groupThreads(filtered, new Date(), props.isPinned ?? isThreadPinned);
  return (
    <ThreadListRoot className="h-full min-h-0">
      <div className="flex-none px-2 pb-2">
        <ThreadListNew
          data-testid="new-thread-button"
          data-analytics-id="chat-sidebar-new-thread"
          className="w-full"
          title={t('chat.newThreadShortcut')}
          label={t('chat.newConversation')}
        />
      </div>
      <div
        data-slot="aui_thread-list-items"
        className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-2 pb-3 [scrollbar-gutter:stable_both-edges] [scrollbar-width:thin]">
        {groups.map(group => (
          <section
            key={group.key}
            data-testid={`thread-group-${group.key}`}
            aria-label={t(GROUP_LABEL_KEYS[group.key])}
            className="flex flex-col gap-0.5">
            <h3
              data-slot="aui_thread-list-group-label"
              className="text-muted-foreground px-2.5 pt-3 pb-1 text-xs font-medium">
              {t(GROUP_LABEL_KEYS[group.key])}
            </h3>
            {group.threads.map(thread => (
              <ThreadListPrimitive.ItemByIndex
                key={thread.id}
                index={threadIds.indexOf(thread.id)}
                components={{ ThreadListItem: ThreadRow }}
              />
            ))}
          </section>
        ))}
        {groups.length === 0 && (
          <p className="text-muted-foreground px-2.5 py-4 text-center text-xs">
            {t('chat.noThreads')}
          </p>
        )}
      </div>
    </ThreadListRoot>
  );
}
