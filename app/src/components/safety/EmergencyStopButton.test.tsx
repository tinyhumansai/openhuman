import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { setHalt } from '../../store/safetySlice';
import { renderWithProviders } from '../../test/test-utils';
import { EmergencyStopButton } from './EmergencyStopButton';

const stop = vi
  .fn()
  .mockResolvedValue({
    engaged: true,
    reason: undefined,
    source: undefined,
    engaged_at_ms: undefined,
  });
vi.mock('../../services/api/emergencyApi', () => ({
  emergencyStop: (...a: unknown[]) => stop(...a),
}));

beforeEach(() => stop.mockClear());

describe('EmergencyStopButton', () => {
  it('renders a button with the emergency stop label', () => {
    renderWithProviders(<EmergencyStopButton />);
    expect(screen.getByRole('button', { name: /emergency stop/i })).toBeDefined();
  });

  it('calls emergencyStop with no argument and dispatches halt on click', async () => {
    const { store } = renderWithProviders(<EmergencyStopButton />);
    fireEvent.click(screen.getByRole('button', { name: /emergency stop/i }));
    await waitFor(() => expect(stop).toHaveBeenCalledWith());
    const safetyState = (store.getState() as { safety: { halted: boolean } }).safety;
    expect(safetyState.halted).toBe(true);
  });

  it('does NOT mark halted when emergencyStop throws, and shows a visible error', async () => {
    stop.mockRejectedValueOnce(new Error('core unavailable'));
    const { store } = renderWithProviders(<EmergencyStopButton />);
    fireEvent.click(screen.getByRole('button', { name: /emergency stop/i }));
    await waitFor(() => expect(stop).toHaveBeenCalled());
    // The core did not confirm the halt, so the UI must not claim halted.
    const safetyState = (store.getState() as { safety: { halted: boolean } }).safety;
    expect(safetyState.halted).toBe(false);
    // Button stays visible so the user can retry.
    expect(screen.queryByRole('button', { name: /emergency stop/i })).not.toBeNull();
    // A visible, retryable error is surfaced so the operator knows it failed.
    await waitFor(() => expect(screen.getByRole('alert')).toBeDefined());
  });

  it('renders nothing while already halted (banner Resume takes over)', () => {
    renderWithProviders(<EmergencyStopButton />, {
      preloadedState: { safety: { halted: true, source: 'user' } },
    });
    expect(screen.queryByRole('button', { name: /emergency stop/i })).toBeNull();
  });

  it('hides itself when the store transitions to halted', async () => {
    const { store } = renderWithProviders(<EmergencyStopButton />);
    expect(screen.getByRole('button', { name: /emergency stop/i })).toBeDefined();
    store.dispatch(setHalt({ source: 'user' }));
    await waitFor(() =>
      expect(screen.queryByRole('button', { name: /emergency stop/i })).toBeNull()
    );
  });
});
