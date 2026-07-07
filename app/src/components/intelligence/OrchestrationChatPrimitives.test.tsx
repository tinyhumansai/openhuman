import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { ChatMessage, ChatWindow } from '../../lib/orchestration/useOrchestrationChats';
import { ChatListButton, MessageBubble } from './OrchestrationChatPrimitives';

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const chat = (over: Partial<ChatWindow>): ChatWindow =>
  ({
    id: 'sess-1',
    kind: 'session',
    title: 'Worker',
    subtitle: 'sub',
    preview: 'last message',
    pinned: false,
    active: true,
    unread: 0,
    lastTimestamp: '2026-07-01T12:00:00.000Z',
    ...over,
  }) as ChatWindow;

describe('ChatListButton', () => {
  it('renders unread count + active badge for an active unread session', () => {
    render(
      <ChatListButton chat={chat({ unread: 4, active: true })} selected onSelect={() => {}} />
    );
    expect(screen.getByText('4')).toBeInTheDocument();
    expect(screen.getByText('tinyplaceOrchestration.active')).toBeInTheDocument();
  });

  it('renders the inactive badge and a contact badge when provided', () => {
    render(
      <ChatListButton
        chat={chat({ active: false })}
        selected={false}
        onSelect={() => {}}
        contactBadge="tinyplaceOrchestration.pairing.linked"
      />
    );
    expect(screen.getByText('tinyplaceOrchestration.inactive')).toBeInTheDocument();
    expect(screen.getByText('tinyplaceOrchestration.pairing.linked')).toBeInTheDocument();
  });

  it('shows the subconscious badge and fires onSelect', () => {
    const onSelect = vi.fn();
    render(
      <ChatListButton chat={chat({ kind: 'subconscious' })} selected={false} onSelect={onSelect} />
    );
    expect(screen.getByText('tinyplaceOrchestration.subconsciousBadge')).toBeInTheDocument();
    fireEvent.click(screen.getByTestId('tinyplace-chat-sess-1'));
    expect(onSelect).toHaveBeenCalled();
  });
});

describe('MessageBubble', () => {
  const message = (over: Partial<ChatMessage>): ChatMessage =>
    ({
      id: 'm1',
      from: '@peer',
      body: 'hello there',
      timestamp: '2026-07-01T12:00:00.000Z',
      encrypted: false,
      ...over,
    }) as ChatMessage;

  it('renders sender + body', () => {
    render(<MessageBubble message={message({})} />);
    expect(screen.getByText('@peer')).toBeInTheDocument();
    expect(screen.getByText('hello there')).toBeInTheDocument();
  });

  it('mutes an encrypted (undecryptable) message body', () => {
    render(<MessageBubble message={message({ encrypted: true, body: '••••' })} />);
    expect(screen.getByText('••••')).toHaveClass('text-content-muted');
  });

  it('renders a tool_call as a monospace command row with ▶ + tool name', () => {
    const { container } = render(
      <MessageBubble message={message({ eventKind: 'tool_call', toolName: 'Bash', body: 'ls' })} />
    );
    expect(container.querySelector('[data-event-kind="tool_call"]')).not.toBeNull();
    expect(screen.getByText('▶')).toBeInTheDocument();
    expect(screen.getByText('Bash')).toBeInTheDocument();
    expect(container.querySelector('p.font-mono')?.textContent).toBe('ls');
  });

  it('renders a tool_result with the ↳ glyph', () => {
    render(
      <MessageBubble
        message={message({ eventKind: 'tool_result', toolName: 'Bash', body: 'ok' })}
      />
    );
    expect(screen.getByText('↳')).toBeInTheDocument();
  });

  it('renders agent_thinking italic + muted with the ∴ glyph', () => {
    const { container } = render(
      <MessageBubble message={message({ eventKind: 'agent_thinking', body: 'considering…' })} />
    );
    expect(container.querySelector('p.italic')).not.toBeNull();
    expect(screen.getByText('∴')).toBeInTheDocument();
  });

  it('renders an error row with the ✕ glyph', () => {
    render(<MessageBubble message={message({ eventKind: 'error', body: 'boom' })} />);
    expect(screen.getByText('✕')).toBeInTheDocument();
  });

  it('falls back to the plain-dot style for a legacy v1 row (no eventKind)', () => {
    const { container } = render(<MessageBubble message={message({ body: 'plain' })} />);
    expect(container.querySelector('[data-event-kind="v1"]')).not.toBeNull();
    expect(container.querySelector('div.rounded-full')).not.toBeNull();
    expect(container.querySelector('p.font-mono')).toBeNull();
  });
});
