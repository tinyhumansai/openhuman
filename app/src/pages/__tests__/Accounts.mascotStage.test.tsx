import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

import Accounts from '../Accounts';

const mockDispatch = vi.fn();

// Flipped by the reduced-motion test; read through the chatMascot mock below.
let reduceMotion = false;

let mascotExpanded = false;
let mascotDismissed = false;
let activeAccountId = '__agent__';

const state = () => ({ accounts: { accounts: {}, order: [], activeAccountId } });

vi.mock('../../store/hooks', () => ({
  useAppDispatch: () => mockDispatch,
  useAppSelector: (selector: (s: ReturnType<typeof state>) => unknown) => selector(state()),
}));
vi.mock('../../store/mascotSlice', () => ({
  selectChatMascotExpanded: () => mascotExpanded,
  selectChatMascotDismissed: () => mascotDismissed,
}));
vi.mock('../../features/conversations/Conversations', () => ({
  ConversationsPage: () => <div data-testid="agent-chat-panel" />,
}));
vi.mock('../../features/human/chatMascot', () => ({
  ChatMascotProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  ChatMascotOverlay: () => <div data-testid="chat-mascot-overlay" />,
  ChatMascotStage: () => <div data-testid="chat-mascot-stage" />,
  MASCOT_TRANSITION_MS: 320,
  prefersReducedMotion: () => reduceMotion,
}));

const renderPage = () =>
  render(
    <MemoryRouter initialEntries={['/chat']}>
      <Accounts />
    </MemoryRouter>
  );

describe('Accounts — merged chat + mascot surface', () => {
  it('collapses the stage column to zero width while the mascot is docked', () => {
    mascotExpanded = false;
    activeAccountId = '__agent__';
    renderPage();

    const column = screen.getByTestId('chat-mascot-stage-column');
    expect(column.style.width).toBe('0px');
    expect(column.dataset.expanded).toBe('false');
  });

  it('unmounts the stage while docked so its controls leave the tab order', () => {
    mascotExpanded = false;
    activeAccountId = '__agent__';
    renderPage();

    expect(screen.queryByTestId('chat-mascot-stage')).not.toBeInTheDocument();
  });

  it('opens the stage column and mounts the stage when expanded', () => {
    mascotExpanded = true;
    activeAccountId = '__agent__';
    renderPage();

    const column = screen.getByTestId('chat-mascot-stage-column');
    // jsdom drops the `min()` width (it does not implement CSS math functions),
    // so assert the column is no longer pinned shut rather than its exact value.
    expect(column.style.width).not.toBe('0px');
    expect(column.dataset.expanded).toBe('true');
    expect(screen.getByTestId('chat-mascot-stage')).toBeInTheDocument();
  });

  it('keeps the transcript mounted in both states', () => {
    // The whole point of the merge: voice and text are one conversation.
    for (const expanded of [false, true]) {
      mascotExpanded = expanded;
      activeAccountId = '__agent__';
      const { unmount } = renderPage();
      expect(screen.getByTestId('agent-chat-panel')).toBeInTheDocument();
      unmount();
    }
  });

  it('drops the column transition when the user prefers reduced motion', () => {
    // The transition is inline (it shares a duration with the mascot's travel),
    // and an inline declaration beats a `motion-reduce:` class — so the
    // preference has to be applied in JS or the column slides while the mascot
    // snaps.
    mascotExpanded = true;
    activeAccountId = '__agent__';
    reduceMotion = true;
    try {
      renderPage();
      expect(screen.getByTestId('chat-mascot-stage-column').style.transition).toBe('');
    } finally {
      reduceMotion = false;
    }
  });

  it('animates the column when reduced motion is not requested', () => {
    mascotExpanded = true;
    activeAccountId = '__agent__';
    renderPage();
    expect(screen.getByTestId('chat-mascot-stage-column').style.transition).toContain('width');
  });

  it('keeps the chat and mascot available when stale provider selection remains', () => {
    mascotExpanded = true;
    activeAccountId = 'acct-whatsapp';
    renderPage();

    expect(screen.getByTestId('chat-mascot-overlay')).toBeInTheDocument();
    expect(screen.getByTestId('chat-mascot-stage')).toBeInTheDocument();
    expect(screen.getByTestId('agent-chat-panel')).toBeInTheDocument();
  });

  it('unmounts the mascot entirely once dismissed', () => {
    // Not merely hidden: a dismissed mascot leaves no anchor to fly to, so the
    // overlay would sit off-screen at opacity 0 with its Rive canvas still
    // re-rendering every lipsync frame and a poll hunting an anchor that never
    // arrives.
    mascotExpanded = false;
    mascotDismissed = true;
    activeAccountId = '__agent__';
    try {
      renderPage();
      expect(screen.queryByTestId('chat-mascot-overlay')).not.toBeInTheDocument();
      expect(screen.getByTestId('agent-chat-panel')).toBeInTheDocument();
    } finally {
      mascotDismissed = false;
    }
  });

  it('keeps the stage shut for a dismissed mascot even if it was left expanded', () => {
    mascotExpanded = true;
    mascotDismissed = true;
    activeAccountId = '__agent__';
    try {
      renderPage();
      expect(screen.getByTestId('chat-mascot-stage-column').style.width).toBe('0px');
      expect(screen.queryByTestId('chat-mascot-stage')).not.toBeInTheDocument();
    } finally {
      mascotDismissed = false;
    }
  });
});
