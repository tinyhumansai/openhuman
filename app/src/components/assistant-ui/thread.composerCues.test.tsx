import {
  AssistantRuntimeProvider,
  type ThreadMessageLike,
  useExternalStoreRuntime,
} from '@assistant-ui/react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { Thread, type ThreadProps } from './thread';

const messages: ThreadMessageLike[] = [
  { role: 'user', content: [{ type: 'text', text: 'hello' }], createdAt: new Date() },
  { role: 'assistant', content: [{ type: 'text', text: 'a reply' }], createdAt: new Date() },
];

function Harness({ isRunning = false, ...props }: { isRunning?: boolean } & ThreadProps) {
  const runtime = useExternalStoreRuntime({
    messages,
    isRunning,
    convertMessage: (m: ThreadMessageLike) => m,
    onNew: async () => {},
    onCancel: async () => {},
  });
  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <Thread {...props} />
    </AssistantRuntimeProvider>
  );
}

function input() {
  return screen.getByTestId('chat-message-input');
}

function type(text: string) {
  const el = input();
  el.textContent = text;
  fireEvent.input(el, { data: text, inputType: 'insertText' });
}

describe('composer cues', () => {
  it('uses the host placeholder when given one', () => {
    render(<Harness composerPlaceholder="Queue a follow-up…" />);
    expect(screen.getByText('Queue a follow-up…')).toBeInTheDocument();
  });

  it('shows Stop on an empty composer and a queue Send once text is typed while running', async () => {
    render(<Harness isRunning />);
    expect(screen.getByTestId('stop-generation-button')).toBeInTheDocument();
    expect(screen.queryByTestId('queue-message-button')).toBeNull();

    type('one more thing');
    await waitFor(() => expect(screen.getByTestId('queue-message-button')).toBeInTheDocument());
    expect(screen.queryByTestId('stop-generation-button')).toBeNull();
  });

  it('lets Escape through when the host did nothing, and swallows it when it acted', () => {
    const idle = vi.fn(() => false);
    const { unmount } = render(<Harness onEscape={idle} />);
    const passthrough = fireEvent.keyDown(input(), { key: 'Escape' });
    expect(idle).toHaveBeenCalledTimes(1);
    expect(passthrough).toBe(true); // not default-prevented
    unmount();

    const acting = vi.fn(() => true);
    render(<Harness onEscape={acting} />);
    const swallowed = fireEvent.keyDown(input(), { key: 'Escape' });
    expect(acting).toHaveBeenCalledTimes(1);
    expect(swallowed).toBe(false);
  });

  it('recalls the last prompt on ArrowUp only in an empty composer without modifiers', async () => {
    const recall = vi.fn(() => true);
    render(<Harness onRecallLastPrompt={recall} />);

    fireEvent.keyDown(input(), { key: 'ArrowUp', shiftKey: true });
    expect(recall).not.toHaveBeenCalled();

    const handled = fireEvent.keyDown(input(), { key: 'ArrowUp' });
    expect(recall).toHaveBeenCalledTimes(1);
    expect(handled).toBe(false);

    type('draft');
    await waitFor(() => expect(input().textContent).toBe('draft'));
    fireEvent.keyDown(input(), { key: 'ArrowUp' });
    expect(recall).toHaveBeenCalledTimes(1);
  });

  it('shows a relative timestamp with the full time as its tooltip', () => {
    render(<Harness />);
    const stamp = screen.getByTestId('message-timestamp');
    expect(stamp).toHaveTextContent(/just now/i);
    expect(stamp.getAttribute('title')).toBeTruthy();
  });

  it('renders the host transcript footer only while nothing runs', () => {
    const Footer = () => <div data-testid="host-footer" />;
    const { unmount } = render(<Harness components={{ TranscriptFooter: Footer }} />);
    expect(screen.getByTestId('host-footer')).toBeInTheDocument();
    unmount();
    render(<Harness isRunning components={{ TranscriptFooter: Footer }} />);
    expect(screen.queryByTestId('host-footer')).toBeNull();
  });
});
