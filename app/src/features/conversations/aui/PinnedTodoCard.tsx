/** Maps the live harness steps onto the upstream AgentPlan presentation. */
import { AgentPlan } from '../../../components/assistant-ui/elements/agent-plan';
import { type TodoItem, todoProgress } from '../../../components/assistant-ui/elements/todo-list';
import { useT } from '../../../lib/i18n/I18nContext';

export function PinnedTodoCard({
  threadId,
  items,
  className,
}: {
  threadId: string;
  items: readonly TodoItem[];
  className?: string;
}) {
  const { t } = useT();
  const progress = todoProgress(items);
  return (
    <AgentPlan
      data-testid="todo-checklist"
      data-thread-id={threadId}
      data-todo-completed={progress.done}
      data-todo-total={progress.total}
      title={t('conversations.runMode.plan')}
      steps={items.map(item => ({
        id: item.id,
        label:
          item.reason && (item.status === 'failed' || item.status === 'skipped')
            ? `${item.text} — ${item.reason}`
            : item.text,
      }))}
      statuses={items.map(item => item.status)}
      activeIndex={progress.done}
      stepTestId="todo-item"
      className={className}
    />
  );
}

export default PinnedTodoCard;
