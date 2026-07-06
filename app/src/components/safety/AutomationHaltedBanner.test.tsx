import { describe, it, expect, vi, beforeEach } from 'vitest';
import { screen, fireEvent, waitFor, act } from '@testing-library/react';
import { AutomationHaltedBanner } from './AutomationHaltedBanner';
import { renderWithProviders } from '../../test/test-utils';
import { setHalt } from '../../store/safetySlice';

const resume = vi.fn().mockResolvedValue({ engaged: false });
vi.mock('../../services/api/emergencyApi', () => ({ emergencyResume: (...a: unknown[]) => resume(...a) }));

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
    expect(screen.getByText('Automation halted')).toBeDefined();
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

  it('clears halt even if emergencyResume throws', async () => {
    resume.mockRejectedValueOnce(new Error('core error'));
    const { store } = renderWithProviders(<AutomationHaltedBanner />, {
      preloadedState: { safety: { halted: true } },
    });
    fireEvent.click(screen.getByRole('button', { name: /resume/i }));
    await waitFor(() => {
      const safetyState = (store.getState() as { safety: { halted: boolean } }).safety;
      expect(safetyState.halted).toBe(false);
    });
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
