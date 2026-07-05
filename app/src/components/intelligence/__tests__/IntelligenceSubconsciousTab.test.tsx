/**
 * Vitest for the Intelligence Subconscious tab.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { SubconsciousInstanceStatus } from '../../../utils/tauriCommands/subconscious';
import IntelligenceSubconsciousTab from '../IntelligenceSubconsciousTab';

const mockNavigate = vi.fn();

function row(instance: 'memory' | 'tinyplace', over: Partial<SubconsciousInstanceStatus> = {}) {
  return {
    instance,
    enabled: true,
    mode: instance === 'memory' ? 'simple' : 'steering',
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

  it('renders both instance cards from instances', () => {
    render(
      <IntelligenceSubconsciousTab
        {...baseProps()}
        mode="simple"
        instances={[row('memory'), row('tinyplace')]}
      />
    );
    expect(screen.getByText('Your world')).toBeInTheDocument();
    expect(screen.getByText('Orchestration steering')).toBeInTheDocument();
    expect(screen.getByText('Run Now')).toBeInTheDocument();
    expect(screen.getByText('Run review now')).toBeInTheDocument();
  });

  it('each card Run button dispatches its own kind', () => {
    const triggerTick = vi.fn().mockResolvedValue(undefined);
    render(
      <IntelligenceSubconsciousTab
        {...baseProps()}
        mode="simple"
        triggerTick={triggerTick}
        instances={[row('memory'), row('tinyplace')]}
      />
    );
    fireEvent.click(screen.getByText('Run Now'));
    expect(triggerTick).toHaveBeenCalledWith('memory');
    fireEvent.click(screen.getByText('Run review now'));
    expect(triggerTick).toHaveBeenCalledWith('tinyplace');
  });

  it('tinyplace card shows a disabled hint when orchestration is off', () => {
    render(
      <IntelligenceSubconsciousTab
        {...baseProps()}
        mode="simple"
        instances={[row('memory'), row('tinyplace', { enabled: false })]}
      />
    );
    expect(screen.getByText(/Enable Orchestration/)).toBeInTheDocument();
    // A disabled card exposes no run button.
    expect(screen.queryByText('Run review now')).not.toBeInTheDocument();
  });

  it('per-kind spinner: only the triggering kind spins', () => {
    render(
      <IntelligenceSubconsciousTab
        {...baseProps()}
        mode="simple"
        instances={[row('memory'), row('tinyplace')]}
        isTriggering={(kind: string) => kind === 'tinyplace'}
      />
    );
    // Memory card's Run button stays enabled; only the tinyplace one is busy.
    expect(screen.getByText('Run Now').closest('button')).toBeEnabled();
    expect(screen.getByText('Run review now').closest('button')).toBeDisabled();
  });
});
