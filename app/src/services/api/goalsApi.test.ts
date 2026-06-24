import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../coreRpcClient';
import { goalsApi } from './goalsApi';

vi.mock('../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const mockCall = vi.mocked(callCoreRpc);

describe('goalsApi', () => {
  beforeEach(() => vi.clearAllMocks());

  it('list returns items from the bare GoalsDoc shape', async () => {
    mockCall.mockResolvedValueOnce({ items: [{ id: 'g1', text: 'ship it' }] });
    const res = await goalsApi.list();
    expect(mockCall).toHaveBeenCalledWith({ method: 'openhuman.memory_goals_list', params: {} });
    expect(res).toEqual([{ id: 'g1', text: 'ship it' }]);
  });

  it('list tolerates a missing/garbage response', async () => {
    mockCall.mockResolvedValueOnce(null);
    expect(await goalsApi.list()).toEqual([]);
  });

  it('add unwraps the { result: { goals: { items } }, logs } envelope', async () => {
    mockCall.mockResolvedValueOnce({
      result: {
        id: 'g2',
        goals: {
          items: [
            { id: 'g1', text: 'a' },
            { id: 'g2', text: 'b' },
          ],
        },
      },
      logs: ['added goal g2'],
    });
    const res = await goalsApi.add('b');
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.memory_goals_add',
      params: { text: 'b' },
    });
    expect(res.map(g => g.id)).toEqual(['g1', 'g2']);
  });

  it('edit unwraps the { result: { items }, logs } envelope', async () => {
    mockCall.mockResolvedValueOnce({ result: { items: [{ id: 'g1', text: 'new' }] }, logs: [] });
    const res = await goalsApi.edit('g1', 'new');
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.memory_goals_edit',
      params: { id: 'g1', text: 'new' },
    });
    expect(res).toEqual([{ id: 'g1', text: 'new' }]);
  });

  it('remove sends the id and returns the updated list', async () => {
    mockCall.mockResolvedValueOnce({ result: { items: [] }, logs: [] });
    const res = await goalsApi.remove('g1');
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.memory_goals_delete',
      params: { id: 'g1' },
    });
    expect(res).toEqual([]);
  });

  it('reflect prunes undefined context and parses ran/summary/items', async () => {
    mockCall.mockResolvedValueOnce({
      result: { ran: true, summary: 'Added 1 goal', goals: { items: [{ id: 'g1', text: 'x' }] } },
      logs: ['reflect complete'],
    });
    const res = await goalsApi.reflect();
    expect(mockCall).toHaveBeenCalledWith(
      expect.objectContaining({ method: 'openhuman.memory_goals_reflect', params: {} })
    );
    expect(res.ran).toBe(true);
    expect(res.summary).toBe('Added 1 goal');
    expect(res.items).toEqual([{ id: 'g1', text: 'x' }]);
  });

  it('reflect forwards a provided context string', async () => {
    mockCall.mockResolvedValueOnce({ result: { ran: false, summary: '', goals: { items: [] } } });
    await goalsApi.reflect('focus on shipping');
    expect(mockCall).toHaveBeenCalledWith(
      expect.objectContaining({ params: { context: 'focus on shipping' } })
    );
  });

  it('reflect parses a bare (un-enveloped) response', async () => {
    mockCall.mockResolvedValueOnce({ ran: true, summary: 'bare', goals: { items: [] } });
    const res = await goalsApi.reflect();
    expect(res.ran).toBe(true);
    expect(res.summary).toBe('bare');
    expect(res.items).toEqual([]);
  });

  it('reflect tolerates a null response', async () => {
    mockCall.mockResolvedValueOnce(null);
    const res = await goalsApi.reflect();
    expect(res).toEqual({ ran: false, summary: '', items: [] });
  });

  it('list returns [] when the payload has neither items nor goals', async () => {
    mockCall.mockResolvedValueOnce({ something: 'else' });
    expect(await goalsApi.list()).toEqual([]);
  });

  it('extractItems drops malformed entries', async () => {
    mockCall.mockResolvedValueOnce({
      items: [{ id: 'g1', text: 'ok' }, { id: 5 }, null, { text: 'no id' }],
    });
    expect(await goalsApi.list()).toEqual([{ id: 'g1', text: 'ok' }]);
  });
});
