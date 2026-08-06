import { describe, expect, test } from 'vitest';

import { buildLinkedInShareUrl, buildTweetIntentUrl, SHARE_LANDING_URL } from './shareTargets';

describe('buildTweetIntentUrl', () => {
  test('encodes caption and url as query params', () => {
    const url = buildTweetIntentUrl('Look what my agent did!', SHARE_LANDING_URL);
    expect(url.startsWith('https://twitter.com/intent/tweet?')).toBe(true);
    const parsed = new URL(url);
    expect(parsed.searchParams.get('text')).toBe('Look what my agent did!');
    expect(parsed.searchParams.get('url')).toBe(SHARE_LANDING_URL);
  });

  test('defaults to the landing url', () => {
    const parsed = new URL(buildTweetIntentUrl('hi'));
    expect(parsed.searchParams.get('url')).toBe(SHARE_LANDING_URL);
  });

  test('trims an over-long caption to leave room for the link', () => {
    const parsed = new URL(buildTweetIntentUrl('a'.repeat(400)));
    const text = parsed.searchParams.get('text') ?? '';
    expect(text.length).toBeLessThanOrEqual(280 - 24);
    expect(text.endsWith('…')).toBe(true);
  });
});

describe('buildLinkedInShareUrl', () => {
  test('builds a share-offsite url with the target url', () => {
    const url = buildLinkedInShareUrl('https://tinyhumans.ai');
    expect(url.startsWith('https://www.linkedin.com/sharing/share-offsite/?')).toBe(true);
    expect(new URL(url).searchParams.get('url')).toBe('https://tinyhumans.ai');
  });

  test('defaults to the landing url', () => {
    expect(new URL(buildLinkedInShareUrl()).searchParams.get('url')).toBe(SHARE_LANDING_URL);
  });
});
