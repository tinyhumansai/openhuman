import { describe, it, expect, vi, beforeEach } from 'vitest';
import { screen, fireEvent, waitFor } from '@testing-library/react';
import { EmergencyStopButton } from './EmergencyStopButton';
import { renderWithProviders } from '../../test/test-utils';

const stop = vi.fn().mockResolvedValue({ engaged: true, reason: undefined, source: undefined, engaged_at_ms: undefined });
vi.mock('../../services/api/emergencyApi', () => ({ emergencyStop: (...a: unknown[]) => stop(...a) }));

beforeEach(() => stop.mockClear());

describe('EmergencyStopButton', () => {
  it('renders a button with the emergency stop label', () => {
    renderWithProviders(<EmergencyStopButton />);
    expect(screen.getByRole('button', { name: /emergency stop/i })).toBeDefined();
  });

  it('calls emergencyStop and dispatches halt on click', async () => {
    const { store } = renderWithProviders(<EmergencyStopButton />);
    fireEvent.click(screen.getByRole('button', { name: /emergency stop/i }));
    await waitFor(() => expect(stop).toHaveBeenCalled());
    const safetyState = (store.getState() as { safety: { halted: boolean } }).safety;
    expect(safetyState.halted).toBe(true);
  });

  it('dispatches halt locally if emergencyStop throws', async () => {
    stop.mockRejectedValueOnce(new Error('core unavailable'));
    const { store } = renderWithProviders(<EmergencyStopButton />);
    fireEvent.click(screen.getByRole('button', { name: /emergency stop/i }));
    await waitFor(() => {
      const safetyState = (store.getState() as { safety: { halted: boolean } }).safety;
      expect(safetyState.halted).toBe(true);
    });
  });
});
