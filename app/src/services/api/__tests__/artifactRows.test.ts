import { describe, expect, it } from 'vitest';

import type { ThreadMessage } from '../../../types/thread';
import { ARTIFACT_IDS_METADATA_KEY, attachArtifactRows } from '../artifactRows';

const user = (id: string, requestId: string): ThreadMessage => ({
  id,
  content: 'make it',
  type: 'text',
  extraMetadata: { requestId },
  sender: 'user',
  createdAt: '2026-10-15T09:00:00Z',
});
const reply = (id: string, requestId?: string): ThreadMessage => ({
  id,
  content: 'Created it.',
  type: 'text',
  extraMetadata: requestId ? { requestId } : {},
  sender: 'agent',
  createdAt: '2026-10-15T09:01:00Z',
});
const row = (artifactId: string, requestId?: string): ThreadMessage => ({
  id: `artifact:${artifactId}`,
  content: artifactId,
  type: 'artifact',
  extraMetadata: requestId ? { artifactId, requestId } : { artifactId },
  sender: 'agent',
  createdAt: '2026-10-15T09:00:30Z',
});
const ids = (message: ThreadMessage) => message.extraMetadata[ARTIFACT_IDS_METADATA_KEY];

describe('attachArtifactRows', () => {
  it('places a backfilled row without a turn on the first reply saved after its artifact', () => {
    const at = (message: ThreadMessage, createdAt: string) => ({ ...message, createdAt });
    // An older thread: replies carry no turn id, and the backfilled row was
    // appended at the end of the log with its artifact's own (earlier) time.
    const out = attachArtifactRows([
      at(user('u1', 'x'), '2026-10-01T09:00:00Z'),
      at(reply('r1'), '2026-10-01T09:01:00Z'),
      at(user('u2', 'y'), '2026-10-01T10:00:00Z'),
      at(reply('r2'), '2026-10-01T10:01:00Z'),
      at(row('old'), '2026-10-01T09:00:40Z'),
    ]);
    expect(out.map(m => m.id)).toEqual(['u1', 'r1', 'u2', 'r2']);
    expect(ids(out[1])).toEqual(['old']);
    expect(ids(out[3])).toBeUndefined();
  });

  it('takes rows out of the list and lists each artifact on its turn reply', () => {
    const out = attachArtifactRows([
      user('u1', 'req-1'),
      row('a', 'req-1'),
      reply('r1', 'req-1'),
      user('u2', 'req-2'),
      row('b', 'req-2'),
      row('c', 'req-2'),
      reply('r2', 'req-2'),
    ]);
    expect(out.map(m => m.id)).toEqual(['u1', 'r1', 'u2', 'r2']);
    expect(ids(out[1])).toEqual(['a']);
    expect(ids(out[3])).toEqual(['b', 'c']);
    expect(ids(out[0])).toBeUndefined();
  });

  it('places a row by its turn even when it was written after later replies (backfill)', () => {
    const out = attachArtifactRows([
      user('u1', 'req-1'),
      reply('r1', 'req-1'),
      user('u2', 'req-2'),
      reply('r2', 'req-2'),
      row('a', 'req-1'),
    ]);
    expect(ids(out[1])).toEqual(['a']);
    expect(ids(out[3])).toBeUndefined();
  });

  it('places a row without a turn on the next agent reply', () => {
    const out = attachArtifactRows([row('a'), reply('r1')]);
    expect(out.map(m => m.id)).toEqual(['r1']);
    expect(ids(out[0])).toEqual(['a']);
  });

  it('drops a row whose turn has no reply yet (the live event places it)', () => {
    const out = attachArtifactRows([user('u1', 'req-1'), row('a', 'req-1')]);
    expect(out.map(m => m.id)).toEqual(['u1']);
  });

  it('keeps message identity when there are no rows', () => {
    const messages = [user('u1', 'req-1'), reply('r1', 'req-1')];
    const out = attachArtifactRows(messages);
    expect(out[0]).toBe(messages[0]);
    expect(out[1]).toBe(messages[1]);
  });
});
