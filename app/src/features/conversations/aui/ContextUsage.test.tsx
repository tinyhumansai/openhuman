import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../../../services/coreRpcClient';
import chatRuntimeReducer, { hydrateThreadUsage } from '../../../store/chatRuntimeSlice';
import { contextBreakdownSegments, ContextUsage, formatCost } from './ContextUsage';

vi.mock('../../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const mockCall = vi.mocked(callCoreRpc);

const BREAKDOWN = {
  agent_id: 'orchestrator',
  model: 'reasoning-v1',
  sections: [
    { label: '(preamble)', bytes: 800, est_tokens: 200 },
    { label: '## Identity', bytes: 400, est_tokens: 100 },
    { label: 'tools', bytes: 8000, est_tokens: 2000 },
    { label: 'history', bytes: 20000, est_tokens: 5000 },
  ],
  tools_bytes: 8000,
  total_est_tokens: 7300,
  context_window: 100000,
};

function renderUsage(
  props: { threadId?: string | null; modelContextWindow?: number | null } = {},
  usage: {
    lastTurnInputTokens: number;
    lastTurnOutputTokens: number;
    lastTurnContextTokens?: number;
    contextWindow: number;
    subAgents?: Array<{
      agentId: string;
      inputTokens: number;
      outputTokens: number;
      costUsd: number;
      runs: number;
    }>;
  } = { lastTurnInputTokens: 40_000, lastTurnOutputTokens: 10_000, contextWindow: 200_000 }
) {
  const store = configureStore({ reducer: combineReducers({ chatRuntime: chatRuntimeReducer }) });
  store.dispatch(
    hydrateThreadUsage({
      threadId: 't1',
      inputTokens: 90_000,
      outputTokens: 20_000,
      cachedTokens: 30_000,
      costUsd: 0.42,
      turns: 3,
      ...usage,
    })
  );
  render(
    <Provider store={store}>
      <ContextUsage threadId={'threadId' in props ? (props.threadId ?? null) : 't1'} {...props} />
    </Provider>
  );
  return store;
}

describe('ContextUsage', () => {
  beforeEach(() => mockCall.mockReset());

  it("renders the ring from the thread's last chat_done usage against its window", () => {
    renderUsage();

    const trigger = screen.getByTestId('composer-context-usage');
    expect(trigger).toHaveAccessibleName('Context usage');
    // 40k in + 10k out of a 200k window.
    expect(trigger).toHaveTextContent('25%');
  });

  it('fills the ring from the last call of a many-call turn, not its summed spend', () => {
    // 5.7M input summed over 72 calls would pin the ring at 100% of a 1M
    // window; the final request held ~100k.
    renderUsage(
      {},
      {
        lastTurnInputTokens: 5_710_657,
        lastTurnOutputTokens: 30_790,
        lastTurnContextTokens: 100_000,
        contextWindow: 1_000_000,
      }
    );

    expect(screen.getByTestId('composer-context-usage')).toHaveTextContent('10%');
  });

  it("prefers the selected model's window over the one the last turn reported", () => {
    renderUsage({ modelContextWindow: 100_000 });

    expect(screen.getByTestId('composer-context-usage')).toHaveTextContent('50%');
  });

  it('renders at 0% before the thread has any usage', () => {
    renderUsage({ threadId: 'fresh-thread' });

    expect(screen.getByTestId('composer-context-usage')).toHaveTextContent('0%');
  });

  it('opens the breakdown on hover without a second tooltip', async () => {
    mockCall.mockResolvedValue(BREAKDOWN);
    renderUsage();

    expect(mockCall).not.toHaveBeenCalled();

    await userEvent.hover(screen.getByTestId('composer-context-usage'));

    expect(mockCall).toHaveBeenCalledTimes(1);
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.agent_context_breakdown',
      params: { thread_id: 't1' },
    });
    const popover = await screen.findByTestId('composer-token-breakdown');
    await waitFor(() => expect(popover).toHaveTextContent('Tool schemas'));
    expect(popover).not.toHaveTextContent('Tool usage');
    expect(popover).not.toHaveTextContent('Thinking tokens');
    expect(popover).not.toHaveTextContent('Output');
    expect(popover).toHaveTextContent('Your input');
    expect(popover).toHaveTextContent('System prompt');
    // Individual system-prompt headings are folded into one stable bucket.
    expect(popover).not.toHaveTextContent('Identity');
    expect(popover).not.toHaveTextContent('## Identity');
    expect(popover).toHaveTextContent('Cache hit');
    expect(popover).toHaveTextContent('33%');
    expect(popover).toHaveTextContent('Cost');
    expect(popover).not.toHaveTextContent('Estimated cost this session');
    expect(document.querySelector('[data-slot="context-display-popover"]')).toBeNull();
    expect(popover).toHaveTextContent('0.4200$');
    expect(popover).toHaveTextContent('Headroom');
    expect(popover).toHaveTextContent('Context window');
    expect(popover).not.toHaveTextContent('conversations.composer');
    // The core's window wins; the buckets use the latest turn's actual usage.
    expect(popover).toHaveTextContent('50k / 100k');
  });

  it('uses compact counts for the context window', async () => {
    mockCall.mockResolvedValue({ ...BREAKDOWN, context_window: 1_048_576 });
    renderUsage(
      {},
      {
        lastTurnInputTokens: 8_511,
        lastTurnOutputTokens: 0,
        lastTurnContextTokens: 8_511,
        contextWindow: 1_048_576,
      }
    );
    await userEvent.hover(screen.getByTestId('composer-context-usage'));
    const breakdown = await screen.findByTestId('composer-token-breakdown');
    await waitFor(() => expect(breakdown).toHaveTextContent('8.5k / 1.0mn'));
  });

  it('includes sub-agent token and cost totals inside the same breakdown card', async () => {
    mockCall.mockResolvedValue(BREAKDOWN);
    renderUsage(
      {},
      {
        lastTurnInputTokens: 40_000,
        lastTurnOutputTokens: 10_000,
        contextWindow: 200_000,
        subAgents: [
          { agentId: 'researcher', inputTokens: 2_500, outputTokens: 500, costUsd: 0.05, runs: 1 },
        ],
      }
    );

    await userEvent.hover(screen.getByTestId('composer-context-usage'));
    const breakdown = await screen.findByTestId('composer-token-breakdown');
    await waitFor(() => expect(breakdown).toHaveTextContent('researcher'));
    expect(breakdown).toHaveTextContent('3,000 · 0.0500$');
    expect(breakdown.querySelectorAll('[data-slot="context-breakdown"]')).toHaveLength(1);
  });

  it('shows an error state instead of crashing when the method is missing, and retries', async () => {
    mockCall.mockRejectedValueOnce(new Error('Method not found'));
    renderUsage();

    await userEvent.hover(screen.getByTestId('composer-context-usage'));

    const popover = await screen.findByTestId('composer-token-breakdown');
    await waitFor(() => expect(popover).toHaveTextContent('Context breakdown unavailable'));
    expect(popover).not.toHaveTextContent('Method not found');

    mockCall.mockResolvedValueOnce(BREAKDOWN);
    await userEvent.click(screen.getByRole('button', { name: 'Try again' }));

    await waitFor(() => expect(popover).toHaveTextContent('Tool schemas'));
    expect(mockCall).toHaveBeenCalledTimes(2);
  });

  it('ignores a response for a request that a reopen superseded', async () => {
    let failFirst: (error: Error) => void = () => {};
    mockCall
      .mockReturnValueOnce(
        new Promise((_, reject) => {
          failFirst = reject;
        })
      )
      .mockResolvedValueOnce(BREAKDOWN);
    renderUsage();
    const trigger = screen.getByTestId('composer-context-usage');

    await userEvent.hover(trigger);
    await screen.findByTestId('composer-token-breakdown');
    await userEvent.unhover(trigger);
    await waitFor(() => expect(screen.queryByTestId('composer-token-breakdown')).toBeNull());
    await userEvent.hover(trigger);
    const popover = await screen.findByTestId('composer-token-breakdown');
    await waitFor(() => expect(popover).toHaveTextContent('Tool schemas'));

    expect(mockCall).toHaveBeenCalledTimes(2);
    await act(async () => failFirst(new Error('late failure')));

    expect(popover).toHaveTextContent('Tool schemas');
    expect(popover).not.toHaveTextContent('Context breakdown unavailable');
  });
});

describe('contextBreakdownSegments', () => {
  const t = (key: string) =>
    ({
      'conversations.composer.context.section.preamble': 'System prompt',
      'conversations.composer.context.section.tools': 'Tools',
      'conversations.composer.context.section.history': 'Conversation history',
      'conversations.composer.context.section.toolSchemas': 'Tool schemas',
      'conversations.composer.context.section.yourInput': 'Your input',
      'conversations.composer.context.output': 'Output',
    })[key] ?? key;

  it('folds repeated headings into one row and drops empty sections', () => {
    const segments = contextBreakdownSegments(
      {
        sections: [
          { label: '## Rules', bytes: 40, est_tokens: 10 },
          { label: '### Rules', bytes: 80, est_tokens: 20 },
          { label: '## Empty', bytes: 0, est_tokens: 0 },
          { label: 'tools', bytes: 400, est_tokens: 100 },
        ],
        total_est_tokens: 130,
        context_window: 0,
      },
      t
    );

    expect(segments.map(s => [s.label, s.tokens])).toEqual([
      ['System prompt', 30],
      ['Tool schemas', 100],
      ['Your input', 0],
    ]);
  });

  it('partitions the final-call context, not the turn’s summed input', () => {
    const segments = contextBreakdownSegments(
      {
        sections: [
          { label: '## Rules', bytes: 40_000, est_tokens: 10_000 },
          { label: 'tools', bytes: 80_000, est_tokens: 20_000 },
        ],
        total_est_tokens: 30_000,
        context_window: 1_000_000,
      },
      t,
      { lastTurnContextUsed: 100_000 }
    );

    // 100k context = 10k prompt + 20k schemas + 70k of conversation. The
    // turn-wide output total is excluded because earlier replies may be gone.
    expect(segments.map(s => [s.label, s.tokens])).toEqual([
      ['System prompt', 10_000],
      ['Tool schemas', 20_000],
      ['Your input', 70_000],
    ]);
    expect(segments.reduce((sum, s) => sum + s.tokens, 0)).toBe(100_000);
  });
});

describe('formatCost', () => {
  it('shows a charge as is, marks an estimate, and shows no price when unknown', () => {
    expect(formatCost(0.2986, 'charged')).toBe('0.2986$');
    expect(formatCost(4.25, 'charged')).toBe('4.25$');
    expect(formatCost(0.05, 'estimated')).toBe('≈ 0.0500$');
    expect(formatCost(4.25, 'unknown')).toBe('—');
  });
});
