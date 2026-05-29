import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { FALLBACK_DEFINITIONS } from '../../lib/channels/definitions';
import { renderChannelIcon } from './channelIcon';

function renderIcon(icon: string, className?: string) {
  return render(<div data-testid="icon-host">{renderChannelIcon(icon, className)}</div>);
}

describe('renderChannelIcon', () => {
  it('renders the Lark / Feishu brand logo as a bundled image', () => {
    const { container } = renderIcon('lark');
    const img = container.querySelector('img');
    expect(img).not.toBeNull();
    expect(img).toHaveAttribute('src', '/lark.png');
  });

  it('renders the DingTalk brand logo as a bundled image', () => {
    const { container } = renderIcon('dingtalk');
    const img = container.querySelector('img');
    expect(img).not.toBeNull();
    expect(img).toHaveAttribute('src', '/dingtalk.png');
  });

  it('renders the Yuanbao brand mark as an inline svg', () => {
    const { container } = renderIcon('yuanbao');
    expect(container.querySelector('svg')).not.toBeNull();
  });

  it('renders the iMessage brand mark as an inline svg', () => {
    const { container } = renderIcon('imessage');
    expect(container.querySelector('svg')).not.toBeNull();
  });

  it.each([
    ['telegram', '✈️'],
    ['discord', '🎮'],
    ['web', '🌐'],
    ['mcp', '🔌'],
  ])('renders the %s channel as its emoji glyph', (icon, glyph) => {
    const { getByTestId } = renderIcon(icon);
    expect(getByTestId('icon-host')).toHaveTextContent(glyph);
  });

  it('renders nothing for an unknown icon so no blank space is reserved', () => {
    const { getByTestId } = renderIcon('totally-unknown-channel');
    expect(getByTestId('icon-host')).toBeEmptyDOMElement();
  });

  it('forwards a custom className to component-backed icons', () => {
    const { container } = renderIcon('lark', 'w-8 h-8');
    expect(container.querySelector('img')).toHaveClass('w-8', 'h-8');
  });

  // Regression guard (issue #2854): every channel the core surfaces
  // (channels/controllers/definitions.rs) plus the virtual MCP tab must render a
  // visible icon — never a blank card. Kept explicit so a newly-added provider
  // without an icon mapping fails here.
  const coreSurfacedIcons = [
    'telegram',
    'discord',
    'web',
    'imessage',
    'lark',
    'dingtalk',
    'yuanbao',
  ];
  const supportedIcons = [
    ...new Set([...coreSurfacedIcons, ...FALLBACK_DEFINITIONS.map(def => def.icon), 'mcp']),
  ];

  it.each(supportedIcons)('renders a non-empty icon for the "%s" channel', icon => {
    const { getByTestId } = renderIcon(icon);
    const host = getByTestId('icon-host');
    const hasVisual =
      host.querySelector('img') !== null ||
      host.querySelector('svg') !== null ||
      (host.textContent ?? '').trim().length > 0;
    expect(hasVisual).toBe(true);
  });
});
