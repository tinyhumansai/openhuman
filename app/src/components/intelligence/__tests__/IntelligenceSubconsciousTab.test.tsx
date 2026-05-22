/**
 * Vitest for the Intelligence Subconscious tab (#623).
 *
 * Covers `handleNavigateToReflectionThread` — the callback passed to
 * `SubconsciousReflectionCards`. The function is small but load-bearing:
 * it dispatches `setSelectedThread(threadId)` so `Conversations` resumes
 * the new thread on mount, then routes to `/chat` (the unified chat
 * surface; `/conversations` redirects to `/home`). Both dispatch and
 * navigate are mocked so we can assert the contract without spinning up
 * the full Redux/router stack.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { setSelectedThread } from '../../../store/threadSlice';
import IntelligenceSubconsciousTab from '../IntelligenceSubconsciousTab';

const mockDispatch = vi.fn();
const mockNavigate = vi.fn();

vi.mock('react-redux', () => ({ useDispatch: () => mockDispatch, useSelector: () => 'en' }));

vi.mock('react-router-dom', () => ({ useNavigate: () => mockNavigate }));

// Stub out the cards component so we can trigger the navigate callback
// directly without exercising the RPC / polling path (already covered by
// `SubconsciousReflectionCards.test.tsx`). The stub renders a button
// that fires `onNavigateToThread` with a known thread id when clicked.
vi.mock('../SubconsciousReflectionCards', () => ({
  default: ({ onNavigateToThread }: { onNavigateToThread?: (id: string) => void }) => (
    <button
      type="button"
      data-testid="cards-stub-trigger"
      onClick={() => onNavigateToThread?.('spawned-thread-42')}>
      trigger
    </button>
  ),
}));

function baseProps() {
  return {
    addSubconsciousTask: vi.fn(),
    approveEscalation: vi.fn(),
    dismissEscalation: vi.fn(),
    expandedLogIds: new Set<string>(),
    logEntries: [],
    newTaskTitle: '',
    removeSubconsciousTask: vi.fn(),
    setExpandedLogIds: vi.fn(),
    setNewTaskTitle: vi.fn(),
    status: null as ComponentProps<typeof IntelligenceSubconsciousTab>['status'],
    tasks: [],
    toggleSubconsciousTask: vi.fn(),
    triggerTick: vi.fn(),
    triggering: false,
    escalations: [],
    loading: false,
  };
}

describe('IntelligenceSubconsciousTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('on Act → dispatches setSelectedThread + navigates to /chat', () => {
    render(<IntelligenceSubconsciousTab {...baseProps()} />);
    fireEvent.click(screen.getByTestId('cards-stub-trigger'));
    // Redux dispatch payload should match the slice's action creator
    // exactly — comparing the produced action keeps the assertion robust
    // if the slice path changes.
    expect(mockDispatch).toHaveBeenCalledWith(setSelectedThread('spawned-thread-42'));
    // Route must be `/chat` (the unified chat surface), not
    // `/conversations` — the latter falls through to a `/home` redirect
    // and the user lands somewhere unexpected.
    expect(mockNavigate).toHaveBeenCalledWith('/chat');
  });

  it('shows provider unavailable state and blocks manual ticks', () => {
    const triggerTick = vi.fn();
    render(
      <IntelligenceSubconsciousTab
        {...baseProps()}
        triggerTick={triggerTick}
        status={{
          enabled: true,
          provider_available: false,
          provider_unavailable_reason: 'Sign in or configure a local Subconscious provider.',
          interval_minutes: 5,
          last_tick_at: null,
          total_ticks: 0,
          task_count: 3,
          pending_escalations: 0,
          consecutive_failures: 1,
        }}
      />
    );

    expect(screen.getByText('Subconscious is paused')).toBeInTheDocument();
    expect(screen.getByText(/configure a local Subconscious provider/i)).toBeInTheDocument();

    const runNow = screen.getByRole('button', { name: /Run Now/i });
    expect(runNow).toBeDisabled();
    fireEvent.click(runNow);
    expect(triggerTick).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: /AI settings/i }));
    expect(mockNavigate).toHaveBeenCalledWith('/settings/llm');
  });
});
