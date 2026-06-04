/**
 * Coverage for the persist filter that gates `chatRuntime.artifactsByThread`
 * on its way to storage (#3024). Confirms the slice's contract:
 * `ready` survives a cold boot, `in_progress` / `failed` do not.
 */
import { describe, expect, it } from 'vitest';

import {
  filterArtifactsForPersist,
  rehydrateArtifactsFromPersist,
} from '../artifactsPersistFilter';
import type { ArtifactSnapshot } from '../chatRuntimeSlice';

function ready(id: string, threadSuffix = ''): ArtifactSnapshot {
  return {
    artifactId: id,
    kind: 'presentation',
    title: `Deck ${id}${threadSuffix}`,
    status: 'ready',
    path: `artifacts/${id}.pptx`,
    sizeBytes: 4096,
    updatedAt: 1700000000000,
  };
}

function inProgress(id: string): ArtifactSnapshot {
  return {
    artifactId: id,
    kind: 'presentation',
    title: `Live ${id}`,
    status: 'in_progress',
    updatedAt: 1700000000000,
  };
}

function failed(id: string, error: string): ArtifactSnapshot {
  return {
    artifactId: id,
    kind: 'presentation',
    title: `Failed ${id}`,
    status: 'failed',
    error,
    updatedAt: 1700000000000,
  };
}

describe('filterArtifactsForPersist (inbound — write side)', () => {
  it('returns an empty map when input is undefined', () => {
    expect(filterArtifactsForPersist(undefined)).toEqual({});
  });

  it('returns an empty map when input has no buckets', () => {
    expect(filterArtifactsForPersist({})).toEqual({});
  });

  it('strips in_progress and failed snapshots, keeps ready ones', () => {
    const inbound = { t1: [ready('a'), inProgress('b'), failed('c', 'timeout')] };
    const out = filterArtifactsForPersist(inbound);
    expect(out.t1).toHaveLength(1);
    expect(out.t1[0].artifactId).toBe('a');
    expect(out.t1[0].status).toBe('ready');
  });

  it('drops bucket keys whose every snapshot was non-ready', () => {
    const inbound = {
      'thread-mixed': [ready('a'), ready('b')],
      'thread-all-in-flight': [inProgress('x'), inProgress('y')],
      'thread-all-failed': [failed('z', 'oops')],
    };
    const out = filterArtifactsForPersist(inbound);
    expect(Object.keys(out)).toEqual(['thread-mixed']);
    expect(out['thread-mixed']).toHaveLength(2);
  });

  it('drops a bucket whose only entry has been deleted (empty list left behind)', () => {
    const inbound = { t1: [] as ArtifactSnapshot[] };
    expect(filterArtifactsForPersist(inbound)).toEqual({});
  });

  it('does not mutate the input map or list', () => {
    const a = ready('a');
    const b = inProgress('b');
    const inbound = { t1: [a, b] };
    const before = JSON.stringify(inbound);
    filterArtifactsForPersist(inbound);
    expect(JSON.stringify(inbound)).toBe(before);
  });

  it('preserves multiple ready snapshots across multiple threads', () => {
    const inbound = { t1: [ready('a'), ready('b')], t2: [ready('c')] };
    const out = filterArtifactsForPersist(inbound);
    expect(out.t1).toHaveLength(2);
    expect(out.t2).toHaveLength(1);
    expect(out.t2[0].artifactId).toBe('c');
  });
});

describe('rehydrateArtifactsFromPersist (outbound — read side)', () => {
  it('passes a populated map through untouched', () => {
    const stored = { t1: [ready('a')] };
    const out = rehydrateArtifactsFromPersist(stored);
    expect(out).toEqual(stored);
  });

  it('substitutes an empty map when storage returns undefined', () => {
    expect(rehydrateArtifactsFromPersist(undefined)).toEqual({});
  });
});
