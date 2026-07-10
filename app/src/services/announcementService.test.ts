import { describe, expect, it, vi } from 'vitest';

import { fetchLatestAnnouncement, parseAnnouncement } from './announcementService';
import { callCoreRpc } from './coreRpcClient';

vi.mock('./coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

describe('parseAnnouncement', () => {
  const valid = {
    id: 'a1',
    title: 'Heads up',
    body: 'Maintenance tonight',
    severity: 'WARNING',
    cta: { label: 'Read more', url: 'https://x.test' },
    startsAt: '2030-01-01T00:00:00.000Z',
    expiresAt: null,
    createdAt: '2030-01-01T00:00:00.000Z',
  };

  it('parses a complete announcement', () => {
    expect(parseAnnouncement(valid)).toEqual({
      id: 'a1',
      title: 'Heads up',
      body: 'Maintenance tonight',
      severity: 'WARNING',
      cta: { label: 'Read more', url: 'https://x.test' },
      startsAt: '2030-01-01T00:00:00.000Z',
      expiresAt: null,
      createdAt: '2030-01-01T00:00:00.000Z',
    });
  });

  it('accepts _id as the id (lean docs)', () => {
    const { id: _omit, ...rest } = valid;
    void _omit;
    expect(parseAnnouncement({ ...rest, _id: 'a2' })?.id).toBe('a2');
  });

  it('returns null for the empty object the backend sends when none is active', () => {
    // parse_api_response_json collapses {success:true,data:null} -> {}.
    expect(parseAnnouncement({})).toBeNull();
  });

  it.each([null, undefined, 'nope', 42])('returns null for non-object %p', raw => {
    expect(parseAnnouncement(raw)).toBeNull();
  });

  it('returns null when title or body is missing', () => {
    expect(parseAnnouncement({ id: 'a1', body: 'b' })).toBeNull();
    expect(parseAnnouncement({ id: 'a1', title: 't' })).toBeNull();
  });

  it('falls back to INFO for an unknown severity', () => {
    expect(parseAnnouncement({ id: 'a1', title: 't', body: 'b', severity: 'LOUD' })?.severity).toBe(
      'INFO'
    );
  });

  it('drops a partial CTA (label or url missing)', () => {
    expect(
      parseAnnouncement({ id: 'a1', title: 't', body: 'b', cta: { label: 'x' } })?.cta
    ).toBeNull();
    expect(
      parseAnnouncement({ id: 'a1', title: 't', body: 'b', cta: { url: 'https://x.test' } })?.cta
    ).toBeNull();
  });
});

describe('fetchLatestAnnouncement', () => {
  it('calls the announcements RPC and parses the result', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ id: 'a1', title: 't', body: 'b' });
    const result = await fetchLatestAnnouncement();
    expect(callCoreRpc).toHaveBeenCalledWith(
      expect.objectContaining({ method: 'openhuman.announcements_get_latest' })
    );
    expect(result?.id).toBe('a1');
  });

  it('returns null when the backend reports no active announcement', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({});
    expect(await fetchLatestAnnouncement()).toBeNull();
  });
});
