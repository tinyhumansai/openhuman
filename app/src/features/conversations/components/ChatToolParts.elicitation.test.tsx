import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { ChatToolFallback } from './ChatToolParts';

const append = vi.fn();
vi.mock('@assistant-ui/react', async importOriginal => ({
  ...(await importOriginal<object>()),
  useAui: () => ({ thread: { append } }),
}));

describe('ChatToolParts ask_user_clarification', () => {
  it('declining sends a reply so the parked turn can continue', () => {
    render(
      <ChatToolFallback
        type="tool-call"
        toolName="ask_user_clarification"
        toolCallId="call-ask"
        args={{ question: 'Which repo?' } as never}
        argsText='{"question":"Which repo?"}'
        status={{ type: 'running' }}
        addResult={() => {}}
        resume={() => {}}
        respondToApproval={async () => {}}
      />
    );

    fireEvent.click(screen.getByRole('button', { name: /decline/i }));
    expect(append).toHaveBeenCalledWith({
      role: 'user',
      content: [{ type: 'text', text: "I'd rather not answer that. Please continue without it." }],
    });
  });
});
