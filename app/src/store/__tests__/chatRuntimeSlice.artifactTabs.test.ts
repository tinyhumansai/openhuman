import { describe, expect, it } from 'vitest';

import reducer, {
  activateArtifactTab,
  clearAllChatRuntime,
  closeArtifactTab,
  openArtifact,
} from '../chatRuntimeSlice';

function seeded() {
  let s = reducer(undefined, openArtifact('a-1'));
  s = reducer(s, openArtifact('a-2'));
  return s;
}

describe('chatRuntimeSlice — artifact tab model (#1535)', () => {
  it('openArtifact adds a tab idempotently and activates it', () => {
    const s = seeded();
    expect(s.openArtifactIds).toEqual(['a-1', 'a-2']);
    expect(s.activeArtifactId).toBe('a-2');

    const s2 = reducer(s, openArtifact('a-1'));
    expect(s2.openArtifactIds).toEqual(['a-1', 'a-2']);
    expect(s2.activeArtifactId).toBe('a-1');
  });

  it('a full runtime reset closes every preview tab', () => {
    const s = reducer(seeded(), clearAllChatRuntime());
    expect(s.openArtifactIds).toEqual([]);
    expect(s.activeArtifactId).toBeNull();
  });

  it('activateArtifactTab switches the active tab', () => {
    const s = reducer(seeded(), activateArtifactTab('a-1'));
    expect(s.activeArtifactId).toBe('a-1');
    expect(s.openArtifactIds).toEqual(['a-1', 'a-2']);
  });

  it('closeArtifactTab removes the tab and activates the next one', () => {
    let s = reducer(seeded(), closeArtifactTab('a-2'));
    expect(s.openArtifactIds).toEqual(['a-1']);
    expect(s.activeArtifactId).toBe('a-1');

    s = reducer(s, closeArtifactTab('a-1'));
    expect(s.openArtifactIds).toEqual([]);
    expect(s.activeArtifactId).toBeNull();
  });

  it('closeArtifactTab of a non-active tab keeps the active tab', () => {
    const s = reducer(seeded(), closeArtifactTab('a-1'));
    expect(s.openArtifactIds).toEqual(['a-2']);
    expect(s.activeArtifactId).toBe('a-2');
  });
});
