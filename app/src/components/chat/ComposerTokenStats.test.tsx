import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen, within } from '@testing-library/react';
import { Provider } from 'react-redux';
import { describe, expect, it, vi } from 'vitest';

import chatRuntimeReducer, { recordChatTurnUsage } from '../../store/chatRuntimeSlice';
import ComposerTokenStats from './ComposerTokenStats';

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

function renderWithUsage(
  payloads: Array<Parameters<typeof recordChatTurnUsage>[0]>,
  props?: { model?: string | null; threadId?: string | null }
) {
  const store = configureStore({ reducer: { chatRuntime: chatRuntimeReducer } });
  for (const p of payloads) store.dispatch(recordChatTurnUsage(p));
  return render(
    <Provider store={store}>
      <ComposerTokenStats {...props} />
    </Provider>
  );
}

const oneTurn = [
  {
    inputTokens: 1200,
    outputTokens: 300,
    cachedTokens: 50,
    costUsd: 0.0123,
    contextWindow: 200_000,
  },
];

describe('<ComposerTokenStats />', () => {
  it('renders nothing before any turn when no model is known', () => {
    const { container } = renderWithUsage([]);
    expect(container).toBeEmptyDOMElement();
  });

  it('renders the clickable context row before the first turn when a model is known', () => {
    renderWithUsage([], { model: 'reasoning-v1' });
    const row = screen.getByRole('button');
    expect(row).toHaveTextContent('token.ctxLabel');
  });

  it('keeps the inline row minimal: context window and cost only', () => {
    renderWithUsage(oneTurn, { model: 'reasoning-v1' });
    const row = screen.getByRole('button');
    // Context window uses the real reported window (200K), not just a default.
    expect(row).toHaveTextContent('token.ctxLabel');
    expect(row).toHaveTextContent('200K');
    // Cost inline.
    expect(row).toHaveTextContent('$0.012');
    // Tokens (in/out) are NOT inline — they live in the popover.
    expect(row).not.toHaveTextContent('token.inLabel');
    expect(row).not.toHaveTextContent('token.outLabel');
    // The model id is NOT inline (it lives in the popover).
    expect(row).not.toHaveTextContent('reasoning-v1');
  });

  it('toggles the breakdown on click and shows explicit labelled rows + tooltips + model', () => {
    renderWithUsage(oneTurn, { model: 'reasoning-v1' });
    expect(screen.queryByTestId('composer-token-breakdown')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button'));
    const bd = screen.getByTestId('composer-token-breakdown');
    // Explicit, spelled-out labels with explanatory tooltips.
    expect(within(bd).getByText('token.popInput')).toHaveAttribute('title', 'token.tipInput');
    expect(within(bd).getByText('token.popOutput')).toHaveAttribute('title', 'token.tipOutput');
    expect(within(bd).getByText('token.popCacheHit')).toHaveAttribute('title', 'token.tipCacheHit');
    // Cache hit shows a hit-rate percentage: 50 cached / 1200 input ≈ 4%.
    expect(within(bd).getByText(/50 \(4%\)/)).toBeInTheDocument();
    // Model id surfaced inside the popover.
    expect(within(bd).getByText('reasoning-v1')).toBeInTheDocument();

    // Clicking again closes it.
    fireEvent.click(screen.getByRole('button'));
    expect(screen.queryByTestId('composer-token-breakdown')).not.toBeInTheDocument();
  });

  it('highlights the context segment while the breakdown is open', () => {
    renderWithUsage(oneTurn);
    const ctx = screen.getByText(/token\.ctxLabel/);
    expect(ctx.className).not.toMatch(/bg-primary/);
    fireEvent.click(screen.getByRole('button'));
    expect(ctx.className).toMatch(/bg-primary/);
  });

  it('closes on Escape and on an outside click', () => {
    renderWithUsage(oneTurn);
    fireEvent.click(screen.getByRole('button'));
    expect(screen.getByTestId('composer-token-breakdown')).toBeInTheDocument();
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(screen.queryByTestId('composer-token-breakdown')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button'));
    expect(screen.getByTestId('composer-token-breakdown')).toBeInTheDocument();
    fireEvent.mouseDown(document.body);
    expect(screen.queryByTestId('composer-token-breakdown')).not.toBeInTheDocument();
  });

  it('breaks down spend per agent: orchestrator (derived) + sub-agents', () => {
    renderWithUsage([
      {
        inputTokens: 500,
        outputTokens: 100,
        costUsd: 0.01,
        subAgents: [{ agentId: 'researcher', inputTokens: 200, outputTokens: 40, costUsd: 0.004 }],
      },
    ]);
    fireEvent.click(screen.getByRole('button'));
    const bd = screen.getByTestId('composer-token-breakdown');
    // Orchestrator = totals − sub-agents: tokens 600−240=360, cost 0.01−0.004=0.006.
    expect(within(bd).getByText('token.orchestrator')).toBeInTheDocument();
    expect(within(bd).getByText(/360/)).toBeInTheDocument();
    expect(within(bd).getByText(/\$0\.006/)).toBeInTheDocument();
    // Sub-agent row: 200 + 40 = 240 combined tokens, its own cost.
    expect(within(bd).getByText('researcher')).toBeInTheDocument();
    expect(within(bd).getByText(/240/)).toBeInTheDocument();
    expect(within(bd).getByText(/\$0\.004/)).toBeInTheDocument();
  });

  it('reads the active thread bucket when a threadId is provided', () => {
    // Two threads with different usage; the footer must reflect the selected one.
    renderWithUsage(
      [
        { inputTokens: 999, outputTokens: 999, costUsd: 0.5, threadId: 'thr-other' },
        { inputTokens: 1200, outputTokens: 300, costUsd: 0.0123, threadId: 'thr-active' },
      ],
      { threadId: 'thr-active' }
    );
    fireEvent.click(screen.getByRole('button'));
    const bd = screen.getByTestId('composer-token-breakdown');
    // Active thread's input tokens (1.2K), not the other thread's 999.
    expect(within(bd).getByText('1.2K')).toBeInTheDocument();
    expect(within(bd).queryByText('999')).not.toBeInTheDocument();
  });

  it('shows the orchestrator row and a no-sub-agents note when none ran', () => {
    renderWithUsage([{ inputTokens: 100, outputTokens: 20, costUsd: 0.001 }]);
    fireEvent.click(screen.getByRole('button'));
    const bd = screen.getByTestId('composer-token-breakdown');
    expect(within(bd).getByText('token.orchestrator')).toBeInTheDocument();
    expect(within(bd).getByText('token.noSubAgents')).toBeInTheDocument();
  });
});
