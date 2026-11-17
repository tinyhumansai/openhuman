import type { McpUiPresentation } from '@/features/conversations/tools/mcpUi/types';
import {
  AssistantRuntimeProvider,
  type ThreadMessageLike,
  useExternalStoreRuntime,
} from '@assistant-ui/react';
import { render, screen } from '@testing-library/react';
import { useEffect } from 'react';
import { describe, expect, it, vi } from 'vitest';

import { Thread } from './thread';

const frames = vi.hoisted(() => ({ mounts: 0 }));

vi.mock('@/utils/tauriCommands/common', async importOriginal => ({
  ...(await importOriginal<typeof import('@/utils/tauriCommands/common')>()),
  isTauri: () => true,
}));

vi.mock('@/features/conversations/tools/mcpUi/McpAppFrame', () => ({
  McpAppFrame: ({ presentation }: { presentation: McpUiPresentation }) => {
    useEffect(() => {
      frames.mounts += 1;
    }, []);
    return (
      <div
        data-testid="mcp-ui-frame"
        data-uri={presentation.resource_uri}
        data-data={JSON.stringify(presentation.structured_content)}
      />
    );
  },
}));

type Part = Exclude<ThreadMessageLike['content'], string>[number];

const widgetCall = (id: string, uri: string, data: unknown): Part =>
  ({
    type: 'tool-call',
    toolCallId: id,
    toolName: 'mcp_shop_tool',
    args: {},
    argsText: '{}',
    result: 'ok',
    artifact: {
      kind: 'openhuman-tool',
      structured: {
        kind: 'mcp_ui',
        flavor: 'mcp_apps',
        server_id: 'shop',
        tool: 'tool',
        resource_uri: uri,
        structured_content: data,
        links: [{ url: 'https://shop.example.com/item/1', kind: 'external' }],
      },
    },
  }) as never;

const turn = (
  content: Part[],
  status: ThreadMessageLike['status'] = { type: 'complete', reason: 'stop' }
): ThreadMessageLike[] => [
  { role: 'user', content: [{ type: 'text', text: 'shop' }] },
  { role: 'assistant', status, content },
];

function Harness({ messages, isRunning }: { messages: ThreadMessageLike[]; isRunning: boolean }) {
  const runtime = useExternalStoreRuntime({
    messages,
    isRunning,
    convertMessage: (m: ThreadMessageLike) => m,
    onNew: async () => {},
  });
  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <Thread />
    </AssistantRuntimeProvider>
  );
}

const frameUris = () =>
  screen.getAllByTestId('mcp-ui-frame').map(frame => frame.getAttribute('data-uri'));

describe('MCP widgets in an assistant message', () => {
  it('render outside the collapsed activity group, before the answer', () => {
    render(
      <Harness
        messages={turn([
          { type: 'reasoning', text: 'looking' },
          widgetCall('t1', 'ui://search', { q: 1 }),
          { type: 'text', text: 'final answer' },
        ])}
        isRunning={false}
      />
    );

    const frame = screen.getByTestId('mcp-ui-frame');
    expect(frame.closest('[data-slot=tool-group-root]')).toBeNull();
    const answer = screen.getByText('final answer');
    expect(frame.compareDocumentPosition(answer) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(screen.queryByTestId('mcp-ui-link')).toBeNull();
  });

  it('show each template once, with its latest data, in order of latest call', () => {
    render(
      <Harness
        messages={turn([
          widgetCall('t1', 'ui://cart', { n: 1 }),
          widgetCall('t2', 'ui://search', { q: 1 }),
          widgetCall('t3', 'ui://cart', { n: 2 }),
          widgetCall('t4', 'ui://cart', { n: 3 }),
          { type: 'text', text: 'done' },
        ])}
        isRunning={false}
      />
    );

    expect(frameUris()).toEqual(['ui://search', 'ui://cart']);
    const cart = screen.getAllByTestId('mcp-ui-frame')[1]!;
    expect(cart.getAttribute('data-data')).toBe(JSON.stringify({ n: 3 }));
  });

  it('keep the same frame mounted when the turn completes', () => {
    frames.mounts = 0;
    const running = turn([widgetCall('t1', 'ui://search', { q: 1 })], { type: 'running' });
    const { rerender } = render(<Harness messages={running} isRunning />);
    expect(screen.getByTestId('mcp-ui-frame')).toBeInTheDocument();

    rerender(
      <Harness
        messages={turn([
          widgetCall('t1', 'ui://search', { q: 1 }),
          { type: 'text', text: 'final answer' },
        ])}
        isRunning={false}
      />
    );

    expect(screen.getAllByTestId('mcp-ui-frame')).toHaveLength(1);
    expect(frames.mounts).toBe(1);
  });
});
