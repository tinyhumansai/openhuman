import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { clearHalt, setHalt } from '../../store/safetySlice';
import { renderWithProviders } from '../../test/test-utils';
import { AutomationHaltedBanner } from './AutomationHaltedBanner';

const resume = vi.fn().mockResolvedValue({ engaged: false });
vi.mock('../../services/api/emergencyApi', () => ({
  emergencyResume: (...a: unknown[]) => resume(...a),
}));

beforeEach(() => resume.mockClear());

describe('AutomationHaltedBanner', () => {
  it('renders nothing when not halted', () => {
    const { container } = renderWithProviders(<AutomationHaltedBanner />);
    expect(container.firstChild).toBeNull();
  });

  it('renders the banner when halted', () => {
    const { store } = renderWithProviders(<AutomationHaltedBanner />, {
      preloadedState: { safety: { halted: true } },
    });
    expect(screen.getByRole('alert')).toBeDefined();
    expect(screen.getByRole('alert').getAttribute('data-analytics-id')).toBe(
      'automation-halted-banner'
    );
    // safety state is engaged
    expect((store.getState() as { safety: { halted: boolean } }).safety.halted).toBe(true);
  });

  it('shows reason when available', () => {
    renderWithProviders(<AutomationHaltedBanner />, {
      preloadedState: { safety: { halted: true, reason: 'custom reason' } },
    });
    expect(screen.getByText('custom reason')).toBeDefined();
  });

  it('falls back to haltedBody when reason is absent', () => {
    renderWithProviders(<AutomationHaltedBanner />, {
      preloadedState: { safety: { halted: true } },
    });
    expect(screen.getByText(/desktop automation is stopped/i)).toBeDefined();
  });

  it('calls emergencyResume and clears halt when Resume is clicked', async () => {
    const { store } = renderWithProviders(<AutomationHaltedBanner />, {
      preloadedState: { safety: { halted: true, reason: 'test' } },
    });
    fireEvent.click(screen.getByRole('button', { name: /resume/i }));
    await waitFor(() => expect(resume).toHaveBeenCalled());
    const safetyState = (store.getState() as { safety: { halted: boolean } }).safety;
    expect(safetyState.halted).toBe(false);
  });

  it('preserves halt and surfaces a retry message when emergencyResume fails', async () => {
    // Fail-closed: on RPC failure the core is still halted, so the UI must
    // NOT silently clear the halt. Clearing locally would re-expose the Stop
    // button while every external-effect action remained blocked, giving a
    // false "resumed" signal (#4255 codex P2).
    resume.mockRejectedValueOnce(new Error('core error'));
    const { store } = renderWithProviders(<AutomationHaltedBanner />, {
      preloadedState: { safety: { halted: true } },
    });
    fireEvent.click(screen.getByRole('button', { name: /resume/i }));
    await waitFor(() => expect(resume).toHaveBeenCalled());
    // Halt state must remain engaged after the failed RPC.
    const safetyState = (store.getState() as { safety: { halted: boolean } }).safety;
    expect(safetyState.halted).toBe(true);
    // Visible retry indicator appears.
    await waitFor(() =>
      expect(screen.getByRole('status', { name: /could not resume/i })).toBeDefined()
    );
    // Banner is still there so the user retains a Resume button to try again.
    expect(screen.getByRole('alert')).toBeDefined();
  });

  it('clears the stale retry indicator on a new halt cycle', async () => {
    // Guards the cross-cycle leak: the banner is mounted permanently, so a failed
    // resume in one cycle must not surface a stale "could not resume" indicator on
    // a later, unrelated halt. Drive: fail a resume → clear the halt via the
    // external socket path (not the successful-RPC branch) → start a fresh halt.
    resume.mockRejectedValueOnce(new Error('core error'));
    const { store } = renderWithProviders(<AutomationHaltedBanner />, {
      preloadedState: { safety: { halted: true } },
    });
    fireEvent.click(screen.getByRole('button', { name: /resume/i }));
    await waitFor(() =>
      expect(screen.getByRole('status', { name: /could not resume/i })).toBeDefined()
    );
    // Halt lifts via the socket-driven clear (bypasses the successful resume path).
    act(() => {
      store.dispatch(clearHalt());
    });
    // A brand-new halt cycle begins.
    act(() => {
      store.dispatch(setHalt({ reason: 'second cycle', source: 'test' }));
    });
    await waitFor(() => expect(screen.getByRole('alert')).toBeDefined());
    // The retry indicator from the previous cycle must not carry over.
    expect(screen.queryByRole('status', { name: /could not resume/i })).toBeNull();
  });

  it('dispatches halt and then renders banner after setHalt dispatch', async () => {
    const { store } = renderWithProviders(<AutomationHaltedBanner />);
    // Initially not halted
    expect((store.getState() as { safety: { halted: boolean } }).safety.halted).toBe(false);
    // Dispatch halt and let React re-render
    act(() => {
      store.dispatch(setHalt({ reason: 'dispatched', source: 'test' }));
    });
    // Banner should appear
    await waitFor(() => expect(screen.getByRole('alert')).toBeDefined());
  });
});
