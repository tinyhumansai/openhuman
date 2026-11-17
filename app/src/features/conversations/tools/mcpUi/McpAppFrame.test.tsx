import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { BridgeOptions } from './mcpAppBridge';
import { McpAppFrame } from './McpAppFrame';
import type { McpUiPresentation } from './types';

const bridges = vi.hoisted(() => ({ options: [] as BridgeOptions[], updates: [] as unknown[] }));
const api = vi.hoisted(() => ({ toolCall: vi.fn(), resourceRead: vi.fn() }));

vi.mock('../../../../services/api/mcpUiApi', () => ({ mcpUiApi: api }));

vi.mock('./mcpAppBridge', () => ({
  McpAppBridge: class {
    constructor(options: BridgeOptions) {
      bridges.options.push(options);
    }
    handleMessage() {
      return false;
    }
    updatePresentation(presentation: McpUiPresentation) {
      bridges.updates.push(presentation.structured_content);
    }
  },
}));

const presentation = (data: unknown = { q: 1 }): McpUiPresentation => ({
  kind: 'mcp_ui',
  flavor: 'mcp_apps',
  server_id: 'shop',
  tool: 'search',
  resource_uri: 'ui://search',
  structured_content: data,
  links: [],
});

async function mountFrame() {
  const view = render(<McpAppFrame presentation={presentation()} />);
  await waitFor(() => expect(bridges.options).toHaveLength(1));
  return { view, handlers: bridges.options[0]!.handlers };
}

beforeEach(() => {
  bridges.options = [];
  bridges.updates = [];
  api.resourceRead.mockResolvedValue({
    html: '<p>widget</p>',
    mime_type: 'text/html;profile=mcp-app',
    csp: { connect_domains: [], resource_domains: [] },
    prefers_border: false,
  });
  api.toolCall.mockImplementation(
    async (_server: string, name: string, _args: unknown, confirmed: boolean) =>
      confirmed
        ? { requires_confirmation: false, read_only: false, result: { tool: name } }
        : { requires_confirmation: true, read_only: false }
  );
});

describe('McpAppFrame tool decisions', () => {
  it('asks once for concurrent calls and remembers Allow for that tool', async () => {
    const { handlers } = await mountFrame();

    let first!: Promise<unknown>;
    let second!: Promise<unknown>;
    act(() => {
      first = handlers.callTool('check_payment', {});
      second = handlers.callTool('check_payment', {});
    });
    await screen.findByTestId('mcp-ui-confirm');
    expect(screen.getAllByTestId('mcp-ui-confirm')).toHaveLength(1);

    fireEvent.click(screen.getByRole('button', { name: 'Allow' }));
    await expect(first).resolves.toEqual({ tool: 'check_payment' });
    await expect(second).resolves.toEqual({ tool: 'check_payment' });

    let third!: Promise<unknown>;
    act(() => {
      third = handlers.callTool('check_payment', {});
    });
    await expect(third).resolves.toEqual({ tool: 'check_payment' });
    expect(screen.queryByTestId('mcp-ui-confirm')).toBeNull();
  });

  it('remembers Deny for that tool and still asks for another tool', async () => {
    const { handlers } = await mountFrame();

    let denied!: Promise<unknown>;
    act(() => {
      denied = handlers.callTool('place_order', {});
    });
    fireEvent.click(await screen.findByRole('button', { name: 'Deny' }));
    await expect(denied).rejects.toThrow();

    let again!: Promise<unknown>;
    act(() => {
      again = handlers.callTool('place_order', {});
    });
    await expect(again).rejects.toThrow();
    expect(screen.queryByTestId('mcp-ui-confirm')).toBeNull();
    expect(api.toolCall).not.toHaveBeenCalledWith('shop', 'place_order', {}, true);

    act(() => {
      void handlers.callTool('update_cart', {}).catch(() => undefined);
    });
    expect(await screen.findByTestId('mcp-ui-confirm')).toBeInTheDocument();
  });

  it('does not reuse an approval for the same tool with different arguments', async () => {
    const { handlers } = await mountFrame();

    let approved!: Promise<unknown>;
    act(() => {
      approved = handlers.callTool('place_order', { item: 'tea' });
    });
    fireEvent.click(await screen.findByRole('button', { name: 'Allow' }));
    await expect(approved).resolves.toEqual({ tool: 'place_order' });

    let sameArgs!: Promise<unknown>;
    act(() => {
      sameArgs = handlers.callTool('place_order', { item: 'tea' });
    });
    await expect(sameArgs).resolves.toEqual({ tool: 'place_order' });
    expect(screen.queryByTestId('mcp-ui-confirm')).toBeNull();

    act(() => {
      void handlers.callTool('place_order', { item: 'laptop' }).catch(() => undefined);
    });
    expect(await screen.findByTestId('mcp-ui-confirm')).toBeInTheDocument();
    expect(api.toolCall).not.toHaveBeenCalledWith('shop', 'place_order', { item: 'laptop' }, true);
  });

  it('queues concurrent confirmations for different calls and answers them in order', async () => {
    const { handlers } = await mountFrame();

    let first!: Promise<unknown>;
    let second!: Promise<unknown>;
    act(() => {
      first = handlers.callTool('place_order', { item: 'tea' });
      second = handlers.callTool('place_order', { item: 'cake' });
    });
    await screen.findByTestId('mcp-ui-confirm');
    expect(screen.getAllByTestId('mcp-ui-confirm')).toHaveLength(1);

    fireEvent.click(screen.getByRole('button', { name: 'Allow' }));
    await expect(first).resolves.toEqual({ tool: 'place_order' });

    await waitFor(() => expect(screen.getByTestId('mcp-ui-confirm')).toBeInTheDocument());
    fireEvent.click(screen.getByRole('button', { name: 'Allow' }));
    await expect(second).resolves.toEqual({ tool: 'place_order' });
    expect(screen.queryByTestId('mcp-ui-confirm')).toBeNull();

    let again!: Promise<unknown>;
    act(() => {
      again = handlers.callTool('place_order', { item: 'tea' });
    });
    await expect(again).resolves.toEqual({ tool: 'place_order' });
    expect(screen.queryByTestId('mcp-ui-confirm')).toBeNull();
  });

  it('rejects still-queued confirmations when the view unmounts', async () => {
    const { view, handlers } = await mountFrame();

    let first!: Promise<unknown>;
    let second!: Promise<unknown>;
    act(() => {
      first = handlers.callTool('place_order', { item: 'tea' });
      second = handlers.callTool('place_order', { item: 'cake' });
    });
    await screen.findByTestId('mcp-ui-confirm');
    const settled = Promise.allSettled([first, second]);
    view.unmount();
    const results = await settled;
    expect(results.map(result => result.status)).toEqual(['rejected', 'rejected']);
  });

  it('keeps one bridge and hands it newer data for the same widget', async () => {
    const { view } = await mountFrame();
    view.rerender(<McpAppFrame presentation={presentation({ q: 2 })} />);

    expect(bridges.options).toHaveLength(1);
    expect(bridges.updates.at(-1)).toEqual({ q: 2 });
  });
});

describe('McpAppFrame sizing', () => {
  it('fills the column, then fits the reported content width, left-aligned', async () => {
    const { handlers } = await mountFrame();
    const frame = screen.getByTestId('mcp-ui-iframe');
    expect(frame.style.width).toBe('100%');
    expect(frame.getAttribute('allowtransparency')).toBe('true');
    expect(frame.getAttribute('src')).toContain('theme=light');
    expect(frame.className).toContain('self-start');

    act(() => handlers.fitWidth(480));
    expect(frame.style.width).toBe('488px');
  });
});
