import { useAuiState } from '@assistant-ui/react';
import { createContext, type PropsWithChildren, useContext, useEffect, useState } from 'react';

import { TaskCard } from '../../../components/assistant-ui/elements/task-card';
import { TodoItems } from '../../../components/assistant-ui/elements/todo-list';
import {
  TaskCardDock,
  TaskCardDockProvider,
} from '../../../components/assistant-ui/lib/task-card-dock';
import { useDisclosure } from '../../../components/assistant-ui/lib/useDisclosure';
import { useT } from '../../../lib/i18n/I18nContext';
import { useAuiThreadId } from '../../../providers/AssistantUiRuntimeProvider';
import { useAppSelector } from '../../../store/hooks';
import { userScopedStorage } from '../../../store/userScopedStorage';
import { taskFinished, taskState, type TurnTask, updateTaskHistory } from './taskCardHistory';
import { toAuiTodoItems } from './TodoListPart';
import { useThreadGoal } from './useThreadGoal';
import { useThreadTodos } from './useThreadTodos';

const Tasks = createContext<TurnTask[]>([]);

/** Presentation snapshots belong to turns, rather than the moving composer. */
export function TurnTaskProvider({ children }: PropsWithChildren) {
  const threadId = useAuiThreadId();
  const todos = useThreadTodos(threadId);
  const goal = useThreadGoal(threadId);
  const anchor = useAppSelector(state => {
    const messages = threadId ? (state.thread.messagesByThreadId[threadId] ?? []) : [];
    return (
      messages.filter(message => message.sender === 'user').at(-1)?.id ?? messages[0]?.id ?? ''
    );
  });
  const [saved, setSaved] = useState<{
    threadId: string | null;
    ready: boolean;
    cards: TurnTask[];
  }>({ threadId: null, ready: false, cards: [] });
  useEffect(() => {
    let cancelled = false;
    if (!threadId) return;
    void userScopedStorage.getItem(`chat-task-cards:${threadId}`).then(value => {
      if (cancelled) return;
      let cards: TurnTask[] = [];
      try {
        const parsed: unknown = JSON.parse(value ?? '[]');
        if (Array.isArray(parsed))
          cards = parsed.filter(
            item => typeof item?.anchor === 'string' && Array.isArray(item?.todos)
          );
      } catch {
        /* Older or unavailable presentation cache. */
      }
      setSaved({ threadId, ready: true, cards });
    });
    return () => {
      cancelled = true;
    };
  }, [threadId]);
  const cards =
    saved.ready && saved.threadId === threadId
      ? updateTaskHistory(saved.cards, { anchor, todos: todos ?? [], goal })
      : [];
  if (saved.ready && saved.threadId === threadId && cards !== saved.cards) {
    setSaved({ ...saved, cards });
  }
  useEffect(() => {
    if (saved.ready && saved.threadId === threadId && threadId) {
      void userScopedStorage.setItem(`chat-task-cards:${threadId}`, JSON.stringify(saved.cards));
    }
  }, [saved, threadId]);
  return (
    <TaskCardDockProvider>
      <Tasks.Provider value={saved.threadId === threadId ? saved.cards : []}>
        {children}
      </Tasks.Provider>
    </TaskCardDockProvider>
  );
}

function TurnTaskCard({ task }: { task: TurnTask }) {
  const { t } = useT();
  const threadId = useAuiThreadId();
  const done = taskFinished(task);
  const [open, setOpen] = useDisclosure(`task:${threadId}:${task.anchor}`, !done);
  const running = useAppSelector(s =>
    Boolean(
      threadId &&
      (s.thread.activeThreadIds[threadId] ||
        s.chatRuntime.pendingSendThreadIds[threadId] ||
        s.chatRuntime.inferenceTurnLifecycleByThread[threadId] === 'started' ||
        s.chatRuntime.inferenceTurnLifecycleByThread[threadId] === 'streaming')
    )
  );
  const state = taskState(task, running);
  const items = toAuiTodoItems(task.todos).map(item =>
    state === 'waiting' && item.status === 'active' ? { ...item, status: 'pending' as const } : item
  );
  return (
    <TaskCard
      data-testid="todo-checklist"
      label={task.goal?.objective ?? t('conversations.runMode.plan')}
      meta={t(`conversations.taskCard.state.${state}`)}
      state={state}
      open={open}
      onOpenChange={setOpen}
      className="max-w-none">
      <TodoItems items={items} />
    </TaskCard>
  );
}

/** Mount only on the first assistant response to the anchored user message. */
export function TurnTaskCards() {
  const cards = useContext(Tasks);
  const anchor = useAuiState(s => {
    const index = s.thread.messages.findIndex(message => message.id === s.message.id);
    let user = '';
    for (let i = 0; i < index; i++) {
      const message = s.thread.messages[i];
      if (message?.role === 'user') user = message.id;
      else if (message?.role === 'assistant') user = '';
    }
    return user || (index === 0 ? s.message.id : '');
  });
  return (
    <>
      {cards
        .filter(task => task.anchor === anchor && taskFinished(task))
        .map(task => (
          <TurnTaskCard key={task.anchor} task={task} />
        ))}
    </>
  );
}

export function ActiveTurnTaskCards() {
  const cards = useContext(Tasks);
  return (
    <TaskCardDock>
      {cards
        .filter(task => !taskFinished(task))
        .map(task => (
          <TurnTaskCard key={task.anchor} task={task} />
        ))}
    </TaskCardDock>
  );
}
