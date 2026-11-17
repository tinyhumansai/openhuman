import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { McpUiBody } from './McpUiBody';

const tauri = vi.hoisted(() => ({ value: false }));

vi.mock('../../../../utils/tauriCommands/common', () => ({ isTauri: () => tauri.value }));

describe('McpUiBody', () => {
  it('renders nothing for other structured payloads', () => {
    const { container } = render(<McpUiBody structured={{ kind: 'web_search' }} />);
    expect(container).toBeEmptyDOMElement();
  });

  it('falls back to links when the widget cannot be sandboxed', () => {
    render(
      <McpUiBody
        structured={{
          kind: 'mcp_ui',
          flavor: 'mcp_apps',
          server_id: 'srv',
          tool: 'checkout',
          resource_uri: 'ui://checkout',
          links: [{ url: 'upi://pay?pa=shop@bank', kind: 'handoff' }],
        }}
      />
    );
    expect(screen.queryByTestId('mcp-ui-frame')).toBeNull();
    expect(screen.getByTestId('mcp-ui-link')).toBeInTheDocument();
  });

  it('leaves a sandboxable widget to the message strip', () => {
    tauri.value = true;
    try {
      render(
        <McpUiBody
          structured={{
            kind: 'mcp_ui',
            flavor: 'mcp_apps',
            server_id: 'srv',
            tool: 'search',
            resource_uri: 'ui://search',
            links: [{ url: 'https://shop.example.com/item/1', kind: 'external' }],
          }}
        />
      );
      expect(screen.queryByTestId('mcp-ui-body')).toBeNull();
      expect(screen.queryByTestId('mcp-ui-frame')).toBeNull();
    } finally {
      tauri.value = false;
    }
  });
});
