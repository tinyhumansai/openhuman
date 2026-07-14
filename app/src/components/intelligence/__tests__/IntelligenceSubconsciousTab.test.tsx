/**
 * Vitest for the Intelligence Subconscious tab.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { SubconsciousInstanceStatus } from '../../../utils/tauriCommands/subconscious';
import IntelligenceSubconsciousTab from '../IntelligenceSubconsciousTab';

const mockNavigate = vi.fn();

function row(instance: 'memory', over: Partial<SubconsciousInstanceStatus> = {}) {
  return {
    instance,
    enabled: true,
    mode: 'simple',
    provider_available: true,
    provider_unavailable_reason: null,
    interval_minutes: 5,
    last_tick_at: null,
    total_ticks: 3,
    consecutive_failures: 0,
    ...over,
  } as SubconsciousInstanceStatus;
}

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

  it('renders the memory instance card from instances', () => {
    render(
      <IntelligenceSubconsciousTab {...baseProps()} mode="simple" instances={[row('memory')]} />
    );
    expect(screen.getByText('Your world')).toBeInTheDocument();
    expect(screen.getByText('Run Now')).toBeInTheDocument();
  });

  it('the memory Run button dispatches the memory kind', () => {
    const triggerTick = vi.fn().mockResolvedValue(undefined);
    render(
      <IntelligenceSubconsciousTab
        {...baseProps()}
        mode="simple"
        triggerTick={triggerTick}
        instances={[row('memory')]}
      />
    );
    fireEvent.click(screen.getByText('Run Now'));
    expect(triggerTick).toHaveBeenCalledWith('memory');
  });
});
