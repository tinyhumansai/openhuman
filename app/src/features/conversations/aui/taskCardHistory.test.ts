import { describe, expect, it } from 'vitest';

import { taskState, type TurnTask, updateTaskHistory } from './taskCardHistory';

const task = (anchor: string, completed = false): TurnTask => ({
  anchor,
  goal: null,
  todos: [{ content: 'Inspect UI', status: completed ? 'completed' : 'in_progress' }],
});
describe('turn task attachment', () => {
  it('derives the visible status from pending, running and completed steps', () => {
    expect(
      taskState({ ...task('turn'), todos: [{ content: 'Inspect UI', status: 'pending' }] })
    ).toBe('waiting');
    expect(taskState(task('turn'))).toBe('working');
    expect(taskState(task('turn'), false)).toBe('waiting');
    expect(
      taskState(
        {
          ...task('turn'),
          goal: {
            threadId: 'turn',
            goalId: 'g',
            objective: 'Plan',
            status: 'paused',
            tokensUsed: 0,
            timeUsedSeconds: 0,
            tokenBudget: null,
            createdAtMs: 1,
            updatedAtMs: 1,
            timeUsedSubsecNanos: 0,
          },
        },
        true
      )
    ).toBe('waiting');
    expect(taskState(task('turn', true))).toBe('done');
  });
  it('keeps completion attached to its original turn as new messages arrive', () => {
    const running = updateTaskHistory([], task('first'));
    const finished = updateTaskHistory(running, task('second', true));
    expect(finished[0]?.anchor).toBe('first');
    expect(updateTaskHistory(finished, task('second', true))).toBe(finished);
    const next = updateTaskHistory(finished, task('second'));
    expect(next.map(item => item.anchor)).toEqual(['first', 'second']);
    expect(next[0]?.todos[0]?.status).toBe('completed');
  });
});
