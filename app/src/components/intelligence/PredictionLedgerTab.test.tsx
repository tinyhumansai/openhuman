import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { PredictionRecord } from '../../lib/memory/predictionLedger';
import PredictionLedgerTab from './PredictionLedgerTab';

const mockLoadLedger = vi.fn();
const mockSaveLedger = vi.fn();

vi.mock('../../services/api/predictionLedgerApi', () => ({
  loadLedger: (...args: unknown[]) => mockLoadLedger(...args),
  saveLedger: (...args: unknown[]) => mockSaveLedger(...args),
}));

const NOW = 1_700_000_000;

function rec(overrides: Partial<PredictionRecord> = {}): PredictionRecord {
  return {
    id: 'p1',
    statement: 'Ships Friday',
    confidence: 0.7,
    createdAt: NOW,
    resolveBy: null,
    outcome: null,
    resolvedAt: null,
    category: null,
    notes: null,
    ...overrides,
  };
}

describe('<PredictionLedgerTab />', () => {
  beforeEach(() => {
    mockLoadLedger.mockReset();
    mockSaveLedger.mockReset();
    mockSaveLedger.mockResolvedValue(undefined);
  });

  it('loads on mount and shows the empty state when there are none', async () => {
    mockLoadLedger.mockResolvedValueOnce([]);
    render(<PredictionLedgerTab onToast={() => {}} />);
    expect(mockLoadLedger).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(screen.getByText('No predictions yet.')).toBeInTheDocument());
  });

  it('surfaces an error alert when loading fails', async () => {
    mockLoadLedger.mockRejectedValueOnce(new Error('ledger unreadable'));
    render(<PredictionLedgerTab onToast={() => {}} />);
    await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/ledger unreadable/));
  });

  it('adds a prediction: persists it and toasts success', async () => {
    mockLoadLedger.mockResolvedValueOnce([]);
    const onToast = vi.fn();
    render(<PredictionLedgerTab onToast={onToast} />);
    await waitFor(() => screen.getByRole('button', { name: 'Add prediction' }));
    fireEvent.change(screen.getByLabelText('Prediction'), { target: { value: 'It rains Sunday' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add prediction' }));
    await waitFor(() => expect(mockSaveLedger).toHaveBeenCalledTimes(1));
    const saved = mockSaveLedger.mock.calls[0][0] as PredictionRecord[];
    expect(saved).toHaveLength(1);
    expect(saved[0].statement).toBe('It rains Sunday');
    expect(saved[0].confidence).toBeCloseTo(0.7, 6); // default 70%
    expect(saved[0].outcome).toBeNull();
    await waitFor(() =>
      expect(onToast).toHaveBeenCalledWith(expect.objectContaining({ type: 'success' }))
    );
  });

  it('resolves an open prediction: persists with the chosen outcome', async () => {
    mockLoadLedger.mockResolvedValueOnce([rec({ id: 'o' })]);
    render(<PredictionLedgerTab onToast={() => {}} />);
    await waitFor(() => screen.getByText('Ships Friday'));
    fireEvent.click(screen.getByRole('button', { name: /as Came true/ }));
    await waitFor(() => expect(mockSaveLedger).toHaveBeenCalledTimes(1));
    const saved = mockSaveLedger.mock.calls[0][0] as PredictionRecord[];
    expect(saved[0].outcome).toBe('true');
    expect(saved[0].resolvedAt).not.toBeNull();
  });

  it('rolls back the optimistic add and toasts when the write fails', async () => {
    mockLoadLedger.mockResolvedValueOnce([]);
    mockSaveLedger.mockRejectedValueOnce(new Error('disk full'));
    const onToast = vi.fn();
    render(<PredictionLedgerTab onToast={onToast} />);
    await waitFor(() => screen.getByRole('button', { name: 'Add prediction' }));
    fireEvent.change(screen.getByLabelText('Prediction'), { target: { value: 'It rains Sunday' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add prediction' }));
    await waitFor(() =>
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({ type: 'error', message: 'disk full' })
      )
    );
    // The optimistic record is reverted -> empty state returns.
    await waitFor(() => expect(screen.getByText('No predictions yet.')).toBeInTheDocument());
  });

  it('deletes a prediction after confirming in the modal', async () => {
    mockLoadLedger.mockResolvedValueOnce([rec({ id: 'r', outcome: 'true', resolvedAt: NOW })]);
    render(<PredictionLedgerTab onToast={() => {}} />);
    await waitFor(() => screen.getByText('Ships Friday'));
    fireEvent.click(screen.getByRole('button', { name: /Delete prediction/ }));
    expect(screen.getByText('Delete prediction')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Confirm' }));
    await waitFor(() => expect(mockSaveLedger).toHaveBeenCalledTimes(1));
    expect(mockSaveLedger.mock.calls[0][0]).toEqual([]);
  });
});
