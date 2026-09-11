import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { createRef } from 'react';
import { describe, expect, it, vi } from 'vitest';

import { Message, MessageAction, MessageActions, MessageContent } from './Message';

const RAW_PALETTE =
  /\b(?:bg|text|border|ring)-(?:neutral|stone|slate|zinc|gray|canvas|white|black)\b/;

const collectClasses = (root: HTMLElement) =>
  [root, ...Array.from(root.querySelectorAll<HTMLElement>('*'))]
    .map(el => el.getAttribute('class') ?? '')
    .join(' ');

describe('Message', () => {
  it('marks a user turn as trailing-aligned', () => {
    render(
      <Message from="user" data-testid="row">
        Hi
      </Message>
    );

    const row = screen.getByTestId('row');
    expect(row).toHaveAttribute('data-slot', 'message');
    expect(row).toHaveAttribute('data-from', 'user');
  });

  it("treats this app's `agent` spelling as the assistant side", () => {
    render(
      <Message from="agent" data-testid="row">
        Hello
      </Message>
    );

    expect(screen.getByTestId('row')).toHaveAttribute('data-from', 'assistant');
  });

  it('forwards a ref to the row element', () => {
    const ref = createRef<HTMLDivElement>();
    render(<Message from="assistant" ref={ref} data-testid="row" />);

    expect(ref.current).toBe(screen.getByTestId('row'));
  });

  it('keeps caller-supplied attributes such as the transcript test hooks', () => {
    render(<Message from="agent" data-testid="chat-message-row" data-sender="agent" />);

    expect(screen.getByTestId('chat-message-row')).toHaveAttribute('data-sender', 'agent');
  });
});

describe('MessageContent', () => {
  it('renders its children in a content slot', () => {
    render(
      <MessageContent data-testid="content">
        <span>Body</span>
      </MessageContent>
    );

    const content = screen.getByTestId('content');
    expect(content).toHaveAttribute('data-slot', 'message-content');
    expect(content).toHaveTextContent('Body');
  });
});

describe('MessageActions', () => {
  it('groups its affordances in an actions slot', () => {
    render(
      <MessageActions data-testid="actions">
        <MessageAction label="Copy">·</MessageAction>
      </MessageActions>
    );

    expect(screen.getByTestId('actions')).toHaveAttribute('data-slot', 'message-actions');
    expect(screen.getByRole('button', { name: 'Copy' })).toBeInTheDocument();
  });
});

describe('MessageAction', () => {
  it('names an icon-only button through its label and fires on click', async () => {
    const user = userEvent.setup();
    const onClick = vi.fn();
    render(
      <MessageAction label="Copy response" onClick={onClick} analyticsId="chat-message-copy">
        <svg />
      </MessageAction>
    );

    const button = screen.getByRole('button', { name: 'Copy response' });
    expect(button).toHaveAttribute('data-slot', 'message-action');
    expect(button).toHaveAttribute('type', 'button');
    expect(button).toHaveAttribute('title', 'Copy response');
    expect(button).toHaveAttribute('data-analytics-id', 'chat-message-copy');

    await user.click(button);
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it('uses no raw palette classes anywhere in the family', () => {
    const { container } = render(
      <Message from="user">
        <MessageContent>
          <MessageActions>
            <MessageAction label="Copy">·</MessageAction>
          </MessageActions>
        </MessageContent>
      </Message>
    );

    expect(collectClasses(container)).not.toMatch(RAW_PALETTE);
  });
});
