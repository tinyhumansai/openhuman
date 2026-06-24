/**
 * Vitest for the Intelligence Subconscious tab.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import IntelligenceSubconsciousTab from '../IntelligenceSubconsciousTab';

const mockNavigate = vi.fn();

vi.mock('react-router-dom', () => ({
  useNavigate: () => mockNavigate,
  useLocation: () => ({
    pathname: '/intelligence',
    search: '',
    hash: '',
    state: null,
    key: 'test',
  }),
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

  it('hides Run Now when mode is off', () => {
    render(<IntelligenceSubconsciousTab {...baseProps()} mode="off" />);
    expect(screen.queryByText('Run Now')).not.toBeInTheDocument();
  });

  it('shows Run Now when mode is simple', () => {
    render(<IntelligenceSubconsciousTab {...baseProps()} mode="simple" />);
    expect(screen.getByText('Run Now')).toBeInTheDocument();
  });

  it('shows aggressive warning when mode is aggressive', () => {
    render(<IntelligenceSubconsciousTab {...baseProps()} mode="aggressive" />);
    expect(screen.getByText(/full tool access including writes/)).toBeInTheDocument();
  });
});
