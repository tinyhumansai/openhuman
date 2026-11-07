import { describe, expect, it } from 'vitest';

import type { Thread } from '../../../types/thread';
import {
  folderBasename,
  groupThreads,
  isThreadPinned,
  labelsWithPin,
  PINNED_THREAD_LABEL,
  recencyGroupFor,
  threadMatchesQuery,
} from './groupThreads';

const NOW = new Date(2026, 9, 6, 15, 0, 0);

function thread(id: string, lastMessageAt: Date, labels: string[] = []): Thread {
  return {
    id,
    title: id,
    chatId: null,
    isActive: true,
    messageCount: 1,
    lastMessageAt: lastMessageAt.toISOString(),
    createdAt: lastMessageAt.toISOString(),
    labels,
  };
}

const daysAgo = (days: number, hour = 12) =>
  new Date(NOW.getFullYear(), NOW.getMonth(), NOW.getDate() - days, hour);

describe('recencyGroupFor', () => {
  it('buckets by local calendar day', () => {
    expect(recencyGroupFor(daysAgo(0, 0).toISOString(), NOW)).toBe('today');
    expect(recencyGroupFor(daysAgo(1, 23).toISOString(), NOW)).toBe('yesterday');
    expect(recencyGroupFor(daysAgo(1, 0).toISOString(), NOW)).toBe('yesterday');
    expect(recencyGroupFor(daysAgo(5).toISOString(), NOW)).toBe('previous7Days');
    expect(recencyGroupFor(daysAgo(20).toISOString(), NOW)).toBe('previous30Days');
    expect(recencyGroupFor(daysAgo(90).toISOString(), NOW)).toBe('older');
  });

  it('sends unparseable timestamps to older', () => {
    expect(recencyGroupFor('not a date', NOW)).toBe('older');
  });
});

describe('groupThreads', () => {
  it('puts pinned threads first and keeps order within sections', () => {
    const groups = groupThreads(
      [
        thread('a', daysAgo(0)),
        thread('p', daysAgo(40), [PINNED_THREAD_LABEL]),
        thread('b', daysAgo(0, 9)),
        thread('c', daysAgo(3)),
      ],
      NOW
    );
    expect(groups.map(g => [g.key, g.threads.map(t => t.id)])).toEqual([
      ['pinned', ['p']],
      ['today', ['a', 'b']],
      ['previous7Days', ['c']],
    ]);
  });

  it('honours a custom pin predicate', () => {
    const groups = groupThreads([thread('a', daysAgo(0))], NOW, () => true);
    expect(groups).toEqual([{ key: 'pinned', threads: [expect.objectContaining({ id: 'a' })] }]);
  });

  it('does not merge project directories sharing a basename', () => {
    const groups = groupThreads(
      [
        { ...thread('project-a', daysAgo(4)), actionDir: '/work/alpha/repo' },
        { ...thread('project-b', daysAgo(0)), actionDir: '/work/beta/repo' },
        { ...thread('project-a2', daysAgo(1)), actionDir: '/work/alpha/repo/' },
        thread('general', daysAgo(0)),
      ],
      NOW
    );

    expect(groups.map(group => group.key)).toEqual([
      'project:/work/alpha/repo',
      'project:/work/beta/repo',
      'today',
    ]);
    expect(groups.map(group => group.label)).toEqual(['repo', 'repo', undefined]);
    expect(groups.map(group => group.threads.map(row => row.id))).toEqual([
      ['project-a', 'project-a2'],
      ['project-b'],
      ['general'],
    ]);
  });

  it('normalizes Windows project spelling but preserves case-sensitive Unix paths', () => {
    const groups = groupThreads(
      [
        { ...thread('windows-a', NOW), actionDir: 'C:\\Work\\Repo\\' },
        { ...thread('windows-b', NOW), actionDir: 'c:/work/repo' },
        { ...thread('unix-a', NOW), actionDir: '/work/Repo' },
        { ...thread('unix-b', NOW), actionDir: '/work/repo' },
      ],
      NOW
    );

    expect(groups.map(group => group.key)).toEqual([
      'project:/work/Repo',
      'project:/work/repo',
      'project:c:\\work\\repo',
    ]);
    const windows = groups.find(group => group.key === 'project:c:\\work\\repo');
    expect(windows?.threads.map(row => row.id)).toEqual(['windows-a', 'windows-b']);
  });

  it('preserves literal backslashes in Unix directory identities and labels', () => {
    const groups = groupThreads(
      [
        { ...thread('plain', NOW), actionDir: '/work/repo' },
        { ...thread('backslash', NOW), actionDir: '/work/repo\\\\' },
      ],
      NOW
    );
    expect(groups).toHaveLength(2);
    expect(groups.map(group => group.key)).toEqual(['project:/work/repo', 'project:/work/repo\\\\']);
    expect(groups.map(group => group.label)).toEqual(['repo', 'repo\\\\']);
  });

  it('retains pinned-first semantics and keeps blank project paths in recency', () => {
    const groups = groupThreads(
      [
        { ...thread('unpinned-project', daysAgo(1)), actionDir: '/work/code' },
        { ...thread('pinned', daysAgo(8), [PINNED_THREAD_LABEL]), actionDir: '/work/code' },
        { ...thread('blank', daysAgo(0)), actionDir: '   ' },
      ],
      NOW
    );

    const groupsAndThreads = groups.map(group => [group.key, group.threads.map(row => row.id)]);
    expect(groupsAndThreads).toEqual([
      ['pinned', ['pinned']],
      ['project:/work/code', ['unpinned-project']],
      ['today', ['blank']],
    ]);
  });

  it('returns no sections for no threads', () => {
    expect(groupThreads([], NOW)).toEqual([]);
  });
});

describe('pin labels', () => {
  it('toggles only the reserved label', () => {
    expect(labelsWithPin(['general'], true)).toEqual(['general', PINNED_THREAD_LABEL]);
    expect(labelsWithPin(['general', PINNED_THREAD_LABEL], true)).toEqual([
      'general',
      PINNED_THREAD_LABEL,
    ]);
    expect(labelsWithPin([PINNED_THREAD_LABEL, 'general'], false)).toEqual(['general']);
    expect(labelsWithPin(undefined, false)).toEqual([]);
    expect(isThreadPinned(thread('x', NOW, [PINNED_THREAD_LABEL]))).toBe(true);
    expect(isThreadPinned(thread('y', NOW))).toBe(false);
  });
});

describe('threadMatchesQuery', () => {
  it('matches case-insensitively and treats blank as all', () => {
    expect(threadMatchesQuery('Fix Gmail OAuth', 'gmail')).toBe(true);
    expect(threadMatchesQuery('Fix Gmail OAuth', '  ')).toBe(true);
    expect(threadMatchesQuery('Fix Gmail OAuth', 'slack')).toBe(false);
  });
});

describe('folderBasename', () => {
  it('returns the last segment on either separator', () => {
    expect(folderBasename('/home/me/projects/site/')).toBe('site');
    expect(folderBasename('C:\\Users\\me\\repo')).toBe('repo');
    expect(folderBasename('/')).toBe('/');
  });
});
