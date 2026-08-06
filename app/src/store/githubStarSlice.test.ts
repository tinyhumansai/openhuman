import { describe, expect, it } from 'vitest';

import reducer, { dismissGithubStarCta, type GithubStarState } from './githubStarSlice';

const initial: GithubStarState = { dismissed: false };

describe('githubStarSlice', () => {
  it('starts undismissed', () => {
    expect(reducer(undefined, { type: '@@INIT' })).toEqual({ dismissed: false });
  });

  it('marks the CTA dismissed', () => {
    const next = reducer(initial, dismissGithubStarCta());
    expect(next.dismissed).toBe(true);
  });

  it('is idempotent — dismissing again stays dismissed', () => {
    const once = reducer(initial, dismissGithubStarCta());
    const twice = reducer(once, dismissGithubStarCta());
    expect(twice.dismissed).toBe(true);
  });
});
