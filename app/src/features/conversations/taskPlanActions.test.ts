import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { TaskBoardCard } from '../../types/turnState';
import { runDecidePlan } from './taskPlanActions';

const mockDecidePlan = vi.fn();
vi.mock('../../services/api/threadApi', () => ({
  threadApi: { decidePlan: (...args: unknown[]) => mockDecidePlan(...args) },
}));

function card(id = 'c1'): TaskBoardCard {
  return { id, title: 'T', status: 'awaiting_approval', order: 0, updatedAt: '' };
}

const t = (key: string) => key;

describe('runDecidePlan', () => {
  beforeEach(() => mockDecidePlan.mockReset());

  it('is a no-op when there is no active thread', async () => {
    const dispatch = vi.fn();
    const notify = vi.fn();

    await runDecidePlan({ threadId: null, card: card(), approve: true, dispatch, notify, t });

    expect(mockDecidePlan).not.toHaveBeenCalled();
    expect(dispatch).not.toHaveBeenCalled();
    expect(notify).not.toHaveBeenCalled();
  });

  it('dispatches the refreshed board on success', async () => {
    mockDecidePlan.mockResolvedValueOnce({ threadId: 't1', cards: [], updatedAt: '' });
    const dispatch = vi.fn();
    const notify = vi.fn();

    await runDecidePlan({ threadId: 't1', card: card(), approve: true, dispatch, notify, t });

    expect(mockDecidePlan).toHaveBeenCalledWith('t1', 'c1', true);
    expect(dispatch).toHaveBeenCalledTimes(1);
    expect(notify).not.toHaveBeenCalled();
  });

  it('does not dispatch when the RPC returns null', async () => {
    mockDecidePlan.mockResolvedValueOnce(null);
    const dispatch = vi.fn();
    const notify = vi.fn();

    await runDecidePlan({ threadId: 't1', card: card(), approve: false, dispatch, notify, t });

    expect(mockDecidePlan).toHaveBeenCalledWith('t1', 'c1', false);
    expect(dispatch).not.toHaveBeenCalled();
    expect(notify).not.toHaveBeenCalled();
  });

  it('notifies (without throwing or dispatching) on RPC failure', async () => {
    mockDecidePlan.mockRejectedValueOnce(new Error('boom'));
    const dispatch = vi.fn();
    const notify = vi.fn();

    await runDecidePlan({ threadId: 't1', card: card(), approve: true, dispatch, notify, t });

    expect(notify).toHaveBeenCalledWith('conversations.taskKanban.updateFailed');
    expect(dispatch).not.toHaveBeenCalled();
  });
});
