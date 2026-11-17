import type { ThreadGoalView } from '../../../store/threadGoalSlice';
import type { ThreadTodoItemView } from '../../../store/threadTodosSlice';

export interface TurnTask {
  anchor: string;
  todos: ThreadTodoItemView[];
  goal: ThreadGoalView | null;
}
export const taskFinished = (task: TurnTask) =>
  Boolean(task.goal || task.todos.length) &&
  (!task.goal || task.goal.status === 'complete') &&
  task.todos.every(item => item.status === 'completed' || item.status === 'cancelled');

export const taskState = (task: TurnTask, running = true): 'done' | 'working' | 'waiting' =>
  taskFinished(task)
    ? 'done'
    : running &&
        task.goal?.status !== 'paused' &&
        task.goal?.status !== 'budget_limited' &&
        (task.goal?.status === 'active' || task.todos.some(item => item.status === 'in_progress'))
      ? 'working'
      : 'waiting';

/** Preserve a completed turn's presentation when later turns begin. */
export function updateTaskHistory(history: TurnTask[], next: TurnTask): TurnTask[] {
  if (!next.anchor || (!next.todos.length && !next.goal)) return history;
  const last = history.at(-1);
  if (
    last &&
    JSON.stringify({ todos: last.todos, goal: last.goal }) ===
      JSON.stringify({ todos: next.todos, goal: next.goal })
  )
    return history;
  if (last && (!taskFinished(last) || last.anchor === next.anchor)) {
    return [...history.slice(0, -1), { ...next, anchor: last.anchor }];
  }
  // A stale completed snapshot must not jump to a new turn.
  if (taskFinished(next)) return last ? history : [next];
  return [...history, next];
}
