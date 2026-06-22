import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockGet = vi.fn();
const mockPost = vi.fn();
const mockPatch = vi.fn();

vi.mock('../apiClient', () => ({
  apiClient: {
    get: (...args: unknown[]) => mockGet(...args),
    post: (...args: unknown[]) => mockPost(...args),
    patch: (...args: unknown[]) => mockPatch(...args),
  },
}));

describe('feedbackApi', () => {
  beforeEach(() => {
    mockGet.mockReset();
    mockPost.mockReset();
    mockPatch.mockReset();
  });

  it('listFeedback builds a query string from params and unwraps data', async () => {
    const payload = { items: [], total: 0, page: 1, limit: 20 };
    mockGet.mockResolvedValueOnce({ success: true, data: payload });

    const { feedbackApi } = await import('./feedbackApi');
    const result = await feedbackApi.listFeedback({
      sort: 'hot',
      type: 'bug',
      status: 'open',
      page: 2,
      limit: 20,
    });

    const url = mockGet.mock.calls[0][0] as string;
    expect(url.startsWith('/feedback?')).toBe(true);
    expect(url).toContain('sort=hot');
    expect(url).toContain('type=bug');
    expect(url).toContain('status=open');
    expect(url).toContain('page=2');
    expect(url).toContain('limit=20');
    expect(result).toEqual(payload);
  });

  it('listFeedback omits the query string when no params are given', async () => {
    mockGet.mockResolvedValueOnce({
      success: true,
      data: { items: [], total: 0, page: 1, limit: 20 },
    });

    const { feedbackApi } = await import('./feedbackApi');
    await feedbackApi.listFeedback();

    expect(mockGet).toHaveBeenCalledWith('/feedback');
  });

  it('submitFeedback posts the payload to /feedback', async () => {
    const result = { accepted: true, reason: '', feedback: null };
    mockPost.mockResolvedValueOnce({ success: true, data: result });

    const { feedbackApi } = await import('./feedbackApi');
    const input = { type: 'feature' as const, title: 'Dark mode', body: 'please' };
    const out = await feedbackApi.submitFeedback(input);

    expect(mockPost).toHaveBeenCalledWith('/feedback', input);
    expect(out).toEqual(result);
  });

  it('voteFeedback posts the vote value to the vote endpoint', async () => {
    mockPost.mockResolvedValueOnce({ success: true, data: { id: 'f1' } });

    const { feedbackApi } = await import('./feedbackApi');
    await feedbackApi.voteFeedback('f1', -1);

    expect(mockPost).toHaveBeenCalledWith('/feedback/f1/vote', { value: -1 });
  });

  it('voteFeedback url-encodes the feedback id', async () => {
    mockPost.mockResolvedValueOnce({ success: true, data: { id: 'a b' } });

    const { feedbackApi } = await import('./feedbackApi');
    await feedbackApi.voteFeedback('a b', 1);

    expect(mockPost).toHaveBeenCalledWith('/feedback/a%20b/vote', { value: 1 });
  });

  it('getFeedback fetches an item with its comments', async () => {
    const detail = { feedback: { id: 'f1' }, comments: [] };
    mockGet.mockResolvedValueOnce({ success: true, data: detail });

    const { feedbackApi } = await import('./feedbackApi');
    const out = await feedbackApi.getFeedback('f1');

    expect(mockGet).toHaveBeenCalledWith('/feedback/f1');
    expect(out).toEqual(detail);
  });

  it('addComment posts the comment body to the comments endpoint', async () => {
    const comment = { id: 'c1', user: 'u1', body: 'nice', createdAt: 'x' };
    mockPost.mockResolvedValueOnce({ success: true, data: comment });

    const { feedbackApi } = await import('./feedbackApi');
    const out = await feedbackApi.addComment('f1', 'nice');

    expect(mockPost).toHaveBeenCalledWith('/feedback/f1/comments', { body: 'nice' });
    expect(out).toEqual(comment);
  });

  it('updateStatus patches the status endpoint (admin)', async () => {
    mockPatch.mockResolvedValueOnce({ success: true, data: { id: 'f1', status: 'planned' } });

    const { feedbackApi } = await import('./feedbackApi');
    await feedbackApi.updateStatus('f1', 'planned');

    expect(mockPatch).toHaveBeenCalledWith('/feedback/f1/status', { status: 'planned' });
  });
});
