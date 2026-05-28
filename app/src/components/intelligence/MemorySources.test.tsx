import { describe, expect, it } from 'vitest';

import type { ComposioConnection } from '../../lib/composio/types';
import type { MemorySyncStatus } from '../../services/memorySyncService';
import { buildRows, isMoreRecentConnection } from './MemorySources';

const SYNCABLE = new Set(['gmail', 'github', 'notion']);

/** Small factory — only the fields the dedupe/render path reads. */
function conn(
  toolkit: string,
  status: string,
  createdAt: string,
  extras: Partial<ComposioConnection> = {}
): ComposioConnection {
  return { id: `${toolkit}-${status}-${createdAt}`, toolkit, status, createdAt, ...extras };
}

describe('isMoreRecentConnection', () => {
  it('picks larger createdAt — regardless of status', () => {
    // The whole point of the fix: a newer EXPIRED supersedes an older
    // ACTIVE, because the new EXPIRED is the user's actual current
    // truth (they re-authorized and that fresh auth then died).
    const olderActive = conn('gmail', 'ACTIVE', '2026-01-01T00:00:00Z');
    const newerExpired = conn('gmail', 'EXPIRED', '2026-05-26T00:00:00Z');
    expect(isMoreRecentConnection(newerExpired, olderActive)).toBe(true);
    expect(isMoreRecentConnection(olderActive, newerExpired)).toBe(false);
  });

  it('a row with createdAt beats a row missing it', () => {
    const dated = conn('gmail', 'EXPIRED', '2026-01-01T00:00:00Z');
    const undated: ComposioConnection = { id: 'x', toolkit: 'gmail', status: 'ACTIVE' };
    expect(isMoreRecentConnection(dated, undated)).toBe(true);
    expect(isMoreRecentConnection(undated, dated)).toBe(false);
  });
});

describe('buildRows', () => {
  const statuses: MemorySyncStatus[] = [];

  it('collapses multiple connections for the same toolkit to one row, picking the newest by createdAt', () => {
    const conns = [
      conn('gmail', 'ACTIVE', '2026-05-26T14:55:00Z'),
      conn('gmail', 'EXPIRED', '2026-05-24T20:13:00Z'),
      conn('gmail', 'EXPIRED', '2026-04-17T08:46:00Z'),
    ];
    const rows = buildRows(conns, statuses, SYNCABLE);
    expect(rows).toHaveLength(1);
    expect(rows[0].toolkit).toBe('gmail');
    expect(rows[0].connection?.status).toBe('ACTIVE');
    expect(rows[0].connection?.createdAt).toBe('2026-05-26T14:55:00Z');
  });

  it('a newer EXPIRED beats an older ACTIVE for the same toolkit', () => {
    // Regression guard: an earlier draft used a status-priority rule
    // that gave ACTIVE/CONNECTED precedence regardless of createdAt,
    // which would zombify a superseded authorization.
    const conns = [
      conn('github', 'ACTIVE', '2025-01-01T00:00:00Z'),
      conn('github', 'EXPIRED', '2026-05-26T14:31:00Z'),
    ];
    const rows = buildRows(conns, statuses, SYNCABLE);
    expect(rows).toHaveLength(1);
    expect(rows[0].connection?.status).toBe('EXPIRED');
    expect(rows[0].connection?.createdAt).toBe('2026-05-26T14:31:00Z');
  });

  it('with no ACTIVE in the group, falls back to the newest non-active state', () => {
    const conns = [
      conn('notion', 'REVOKED', '2026-05-20T09:06:00Z'),
      conn('notion', 'EXPIRED', '2026-04-17T08:48:00Z'),
      conn('notion', 'EXPIRED', '2026-04-17T08:47:00Z'),
    ];
    const rows = buildRows(conns, statuses, SYNCABLE);
    expect(rows).toHaveLength(1);
    expect(rows[0].connection?.status).toBe('REVOKED');
    expect(rows[0].connection?.createdAt).toBe('2026-05-20T09:06:00Z');
  });

  it('keeps distinct accounts on the same toolkit separate when identity is populated', () => {
    // Once the backend ships identity fields (accountEmail/workspace/
    // username), two genuinely different gmail accounts must NOT
    // collapse into one row.
    const conns = [
      conn('gmail', 'ACTIVE', '2026-05-26T00:00:00Z', { accountEmail: 'alice@x.com' }),
      conn('gmail', 'EXPIRED', '2026-05-25T00:00:00Z', { accountEmail: 'alice@x.com' }),
      conn('gmail', 'ACTIVE', '2026-04-01T00:00:00Z', { accountEmail: 'bob@y.com' }),
    ];
    const rows = buildRows(conns, statuses, SYNCABLE);
    expect(rows).toHaveLength(2);
    const aliceRow = rows.find(r => r.connection?.accountEmail === 'alice@x.com');
    const bobRow = rows.find(r => r.connection?.accountEmail === 'bob@y.com');
    expect(aliceRow?.connection?.status).toBe('ACTIVE');
    expect(aliceRow?.connection?.createdAt).toBe('2026-05-26T00:00:00Z');
    expect(bobRow?.connection?.status).toBe('ACTIVE');
  });

  it('drops connections whose toolkit is not in the syncable set', () => {
    const conns = [
      conn('googledrive', 'EXPIRED', '2026-05-20T00:00:00Z'),
      conn('gmail', 'ACTIVE', '2026-05-26T00:00:00Z'),
    ];
    const rows = buildRows(conns, statuses, SYNCABLE);
    expect(rows).toHaveLength(1);
    expect(rows[0].toolkit).toBe('gmail');
  });

  it('attaches the matching MemorySyncStatus to each row by toolkit name', () => {
    const conns = [conn('gmail', 'ACTIVE', '2026-05-26T00:00:00Z')];
    const fakeStatus = {
      provider: 'gmail',
      freshness: 'idle',
      last_chunk_at_ms: 0,
      chunks_synced: 2,
      chunks_pending: 0,
      batch_total: 0,
      batch_processed: 0,
    } as unknown as MemorySyncStatus;
    const rows = buildRows(conns, [fakeStatus], SYNCABLE);
    expect(rows[0].status).toBe(fakeStatus);
  });
});
