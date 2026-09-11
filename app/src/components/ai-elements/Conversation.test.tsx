import { render, screen } from '@testing-library/react';
import { createRef } from 'react';
import { describe, expect, it } from 'vitest';

import { Conversation, ConversationContent } from './Conversation';

const RAW_PALETTE =
  /\b(?:bg|text|border|ring)-(?:neutral|stone|slate|zinc|gray|canvas|white|black)\b/;

const collectClasses = (root: HTMLElement) =>
  [root, ...Array.from(root.querySelectorAll<HTMLElement>('*'))]
    .map(el => el.getAttribute('class') ?? '')
    .join(' ');

describe('Conversation', () => {
  it('exposes the scroll viewport as a log region', () => {
    render(<Conversation data-testid="viewport" />);

    const viewport = screen.getByTestId('viewport');
    expect(viewport).toHaveAttribute('data-slot', 'conversation');
    expect(viewport).toHaveAttribute('role', 'log');
  });

  it('forwards its ref, which is how the host wires stick-to-bottom', () => {
    const ref = createRef<HTMLDivElement>();
    render(<Conversation ref={ref} data-testid="viewport" />);

    expect(ref.current).toBe(screen.getByTestId('viewport'));
  });

  it('renders the turn column and its children', () => {
    render(
      <Conversation>
        <ConversationContent data-testid="content">
          <span>One turn</span>
        </ConversationContent>
      </Conversation>
    );

    const content = screen.getByTestId('content');
    expect(content).toHaveAttribute('data-slot', 'conversation-content');
    expect(content).toHaveTextContent('One turn');
  });

  it('keeps a caller className alongside the shell classes', () => {
    render(<ConversationContent data-testid="content" className="px-5" />);

    expect(screen.getByTestId('content').getAttribute('class')).toContain('px-5');
  });

  it('uses no raw palette classes', () => {
    const { container } = render(
      <Conversation>
        <ConversationContent />
      </Conversation>
    );

    expect(collectClasses(container)).not.toMatch(RAW_PALETTE);
  });
});
