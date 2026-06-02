/**
 * Vitest for the Intelligence Subconscious tab.
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

function baseProps(): ComponentProps<typeof IntelligenceSubconsciousTab> {
  return {
    status: null,
    mode: 'off',
    intervalMinutes: 30,
    triggerTick: vi.fn(),
    triggering: false,
    settingMode: false,
    setMode: vi.fn(),
    setIntervalMinutes: vi.fn(),
  };
}

describe('IntelligenceSubconsciousTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('renders three mode options', () => {
    render(<IntelligenceSubconsciousTab {...baseProps()} />);
    expect(screen.getByText('Off')).toBeInTheDocument();
    expect(screen.getByText('Simple')).toBeInTheDocument();
    expect(screen.getByText('Aggressive')).toBeInTheDocument();
  });

  it('clicking a mode option calls setMode', () => {
    const setMode = vi.fn();
    render(<IntelligenceSubconsciousTab {...baseProps()} setMode={setMode} />);
    fireEvent.click(screen.getByText('Simple'));
    expect(setMode).toHaveBeenCalledWith('simple');
  });

  it('hides Run Now and reflections when mode is off', () => {
    render(<IntelligenceSubconsciousTab {...baseProps()} mode="off" />);
    expect(screen.queryByText('Run Now')).not.toBeInTheDocument();
    expect(screen.queryByTestId('cards-stub-trigger')).not.toBeInTheDocument();
  });

  it('shows Run Now and reflections when mode is simple', () => {
    render(<IntelligenceSubconsciousTab {...baseProps()} mode="simple" />);
    expect(screen.getByText('Run Now')).toBeInTheDocument();
    expect(screen.getByTestId('cards-stub-trigger')).toBeInTheDocument();
  });

  it('shows aggressive warning when mode is aggressive', () => {
    render(<IntelligenceSubconsciousTab {...baseProps()} mode="aggressive" />);
    expect(screen.getByText(/full tool access including writes/)).toBeInTheDocument();
  });

  it('on Act → dispatches setSelectedThread + navigates to /chat', () => {
    render(<IntelligenceSubconsciousTab {...baseProps()} mode="simple" />);
    fireEvent.click(screen.getByTestId('cards-stub-trigger'));
    expect(mockDispatch).toHaveBeenCalledWith(setSelectedThread('spawned-thread-42'));
    expect(mockNavigate).toHaveBeenCalledWith('/chat');
  });
});
