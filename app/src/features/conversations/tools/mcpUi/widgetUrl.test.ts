import { describe, expect, it } from 'vitest';

import { FILL_WIDTH } from './frameFit';
import { frameWidth, PROXY_FRAME_SANDBOX, widgetProxyUrl } from './widgetUrl';

describe('widgetProxyUrl', () => {
  it('uses the platform origin and carries the declared origins', () => {
    expect(widgetProxyUrl(undefined, false)).toBe('ohwidget://localhost/proxy');
    expect(
      widgetProxyUrl(
        {
          connect_domains: ['https://api.example.com'],
          resource_domains: ['https://cdn.example.com'],
        },
        true
      )
    ).toBe(
      'http://ohwidget.localhost/proxy?connect=https%3A%2F%2Fapi.example.com&resource=https%3A%2F%2Fcdn.example.com'
    );
  });

  it('carries the host theme', () => {
    expect(widgetProxyUrl(undefined, false, 'dark')).toBe('ohwidget://localhost/proxy?theme=dark');
  });
});

describe('frameWidth', () => {
  it('fills the column until a content width is known, or when content fills', () => {
    expect(frameWidth(null)).toBe('100%');
    expect(frameWidth(Number.NaN)).toBe('100%');
    expect(frameWidth(FILL_WIDTH)).toBe('100%');
  });

  it('fits the content with a margin, never below the minimum', () => {
    expect(frameWidth(480)).toBe('488px');
    expect(frameWidth(479.2)).toBe('488px');
    expect(frameWidth(100)).toBe('320px');
  });
});

describe('PROXY_FRAME_SANDBOX', () => {
  it('gives the proxy an origin a widget can post to, and nothing else', () => {
    expect(PROXY_FRAME_SANDBOX.split(' ').sort()).toEqual([
      'allow-forms',
      'allow-same-origin',
      'allow-scripts',
    ]);
  });
});
