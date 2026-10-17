import type { ToolCallMessagePartComponent } from '@assistant-ui/react';

import {
  type TodoItem,
  todoProgress,
  TodoReceipt,
  type TodoStatus,
} from '../../../components/assistant-ui/elements/todo-list';
import { useDisclosure } from '../../../components/assistant-ui/lib/useDisclosure';
import { useT } from '../../../lib/i18n/I18nContext';
import type { CoreTodoStatus } from '../../../store/threadTodosSlice';

/**
 * Adapts the core wire shape of a `todo` tool call — `{content, status,
 * reason?}` with `status: 'pending'|'in_progress'|'completed'|'cancelled'` —
 * onto the vendored `TodoList` element's `TodoItem` shape (`{id, text, status,
 * reason?}` with `status: 'pending'|'active'|'done'|'failed'|'skipped'`). A
 * cancelled step shows as skipped, never as pending. The core never sends
 * `'failed'`, so that status never maps.
 */
export function mapCoreTodoStatus(status: unknown): TodoStatus {
  switch (status) {
    case 'in_progress':
      return 'active';
    case 'completed':
      return 'done';
    case 'cancelled':
      return 'skipped';
    case 'pending':
      return 'pending';
    default:
      return 'pending';
  }
}

interface CoreTodoItem {
  content?: unknown;
  status?: unknown;
  reason?: unknown;
}

function isCoreTodoItem(value: unknown): value is CoreTodoItem {
  return Boolean(value) && typeof value === 'object';
}

/** `content`+index as a stable id when the payload has no id field of its own. */
export function toAuiTodoItems(raw: unknown): TodoItem[] {
  if (!Array.isArray(raw)) return [];
  const items: TodoItem[] = [];
  raw.forEach((candidate, index) => {
    if (!isCoreTodoItem(candidate)) return;
    const content = typeof candidate.content === 'string' ? candidate.content : '';
    if (!content.trim()) return;
    items.push({
      id: `${index}-${content}`,
      text: content,
      status: mapCoreTodoStatus(candidate.status),
      ...(typeof candidate.reason === 'string' && candidate.reason
        ? { reason: candidate.reason }
        : {}),
    });
  });
  return items;
}

/** Args/result shape the core sends for the `todo` tool call. */
interface TodoToolArgs {
  todos?: Array<{ content: string; status: CoreTodoStatus }>;
}

/**
 * Toolkit render for the `todo` tool call — a one-line receipt in the
 * transcript ("Progress updated — 3/7 · current step") that expands to the
 * todo list snapshot as of THIS call (args carry the write while in flight;
 * result echoes it back once settled). A run updates its list many times, and
 * repeating the whole list after every update buried the conversation. The
 * pinned, always-current list above the composer is a separate render
 * (`PinnedTodoCard`), driven by {@link useThreadTodos} off the live
 * `thread_todos_changed` event, not this per-call snapshot.
 */
export const TodoListPart: ToolCallMessagePartComponent = ({ args, result, toolCallId }) => {
  const { t } = useT();
  const [open, setOpen] = useDisclosure(toolCallId ? `todo:${toolCallId}` : undefined, false);
  const payload = (result ?? args) as TodoToolArgs | undefined;
  const items = toAuiTodoItems(payload?.todos);
  if (items.length === 0) return null;
  const progress = todoProgress(items);
  return (
    <div data-testid="todo-receipt">
      <TodoReceipt
        items={items}
        open={open}
        onOpenChange={setOpen}
        label={t('chat.todos.receipt')
          .replace('{done}', String(progress.done))
          .replace('{total}', String(progress.total))}
      />
    </div>
  );
};
