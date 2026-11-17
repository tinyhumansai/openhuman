import { describe, expect, it } from 'vitest';

import { hasWidget, isImageUrl, readMcpUiPresentation } from './types';

describe('readMcpUiPresentation', () => {
  it('reads a presentation and drops malformed links', () => {
    const presentation = readMcpUiPresentation({
      kind: 'mcp_ui',
      flavor: 'apps_sdk',
      server_id: 'srv',
      tool: 'cart',
      resource_uri: 'ui://cart',
      links: [{ url: 'upi://pay', kind: 'handoff' }, { url: 3 }, 'x'],
    });
    expect(presentation?.flavor).toBe('apps_sdk');
    expect(presentation?.links).toEqual([{ url: 'upi://pay', kind: 'handoff' }]);
    expect(presentation && hasWidget(presentation)).toBe(true);
  });

  it('rejects other kinds', () => {
    expect(readMcpUiPresentation({ kind: 'web_search' })).toBeNull();
    expect(readMcpUiPresentation(null)).toBeNull();
    expect(readMcpUiPresentation([{ kind: 'mcp_ui' }])).toBeNull();
  });

  it('needs a server for a resource widget but not for an inline one', () => {
    const resource = readMcpUiPresentation({ kind: 'mcp_ui', tool: 't', resource_uri: 'ui://a' });
    expect(resource && hasWidget(resource)).toBe(false);
    const inline = readMcpUiPresentation({ kind: 'mcp_ui', tool: 'show_ui', inline_id: 'abc' });
    expect(inline && hasWidget(inline)).toBe(true);
  });
});

describe('image links', () => {
  it('drops image assets but keeps pages and app links', () => {
    const presentation = readMcpUiPresentation({
      kind: 'mcp_ui',
      tool: 'search',
      links: [
        {
          url: 'https://media-assets.swiggy.com/swiggy/image/upload/fl_lossy/NI_CATALOG/x_1',
          kind: 'external',
        },
        { url: 'https://cdn.example.com/a/bar.JPG', kind: 'external' },
        { url: 'https://shop.example.com/item/1', kind: 'external' },
        { url: 'upi://pay?pa=a@b.png', kind: 'handoff' },
      ],
    });
    expect(presentation?.links.map(link => link.url)).toEqual([
      'https://shop.example.com/item/1',
      'upi://pay?pa=a@b.png',
    ]);
  });

  it('classifies by path only', () => {
    expect(isImageUrl('https://res.cloudinary.com/demo/image/upload/sample')).toBe(true);
    expect(isImageUrl('https://cdn.example.com/logo.svg?v=2')).toBe(true);
    expect(isImageUrl('https://example.com/pngs/list')).toBe(false);
    expect(isImageUrl('not a url')).toBe(false);
  });
});
