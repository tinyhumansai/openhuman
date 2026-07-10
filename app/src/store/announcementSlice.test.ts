import { describe, expect, it } from 'vitest';

import reducer, { type AnnouncementState, markAnnouncementShown } from './announcementSlice';

const initial: AnnouncementState = { shownIds: [] };

describe('announcementSlice', () => {
  it('records a shown announcement id', () => {
    const next = reducer(initial, markAnnouncementShown('a1'));
    expect(next.shownIds).toEqual(['a1']);
  });

  it('does not duplicate an already-seen id', () => {
    const once = reducer(initial, markAnnouncementShown('a1'));
    const twice = reducer(once, markAnnouncementShown('a1'));
    expect(twice.shownIds).toEqual(['a1']);
  });

  it('ignores an empty id', () => {
    expect(reducer(initial, markAnnouncementShown('')).shownIds).toEqual([]);
  });

  it('caps the history at 200, dropping the oldest', () => {
    let state = initial;
    for (let i = 0; i < 205; i += 1) {
      state = reducer(state, markAnnouncementShown(`id-${i}`));
    }
    expect(state.shownIds).toHaveLength(200);
    expect(state.shownIds[0]).toBe('id-5');
    expect(state.shownIds.at(-1)).toBe('id-204');
  });
});
