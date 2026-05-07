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
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { setSelectedThread } from '../../../store/threadSlice';
import IntelligenceSubconsciousTab from '../IntelligenceSubconsciousTab';

const mockDispatch = vi.fn();
const mockNavigate = vi.fn();

vi.mock('react-redux', () => ({ useDispatch: () => mockDispatch }));

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
    status: null,
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
});
