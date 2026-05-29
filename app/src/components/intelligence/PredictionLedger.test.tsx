import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { PredictionOutcome, PredictionRecord } from '../../lib/memory/predictionLedger';
import PredictionLedger from './PredictionLedger';

const NOW = 1_700_000_000;

let seq = 0;
function rec(overrides: Partial<PredictionRecord> = {}): PredictionRecord {
  seq += 1;
  return {
    id: `p${seq}`,
    statement: 'The deploy ships Friday',
    confidence: 0.6,
    createdAt: NOW,
    resolveBy: null,
    outcome: null,
    resolvedAt: null,
    category: null,
    notes: null,
    ...overrides,
  };
}

const resolvedFixture: PredictionRecord[] = [
  rec({ id: 'a', confidence: 0.8, outcome: 'true', resolvedAt: NOW }),
  rec({ id: 'b', confidence: 0.3, outcome: 'false', resolvedAt: NOW }),
];

describe('<PredictionLedger />', () => {
  it('renders a loading skeleton', () => {
    const { container } = render(<PredictionLedger records={[]} loading nowSeconds={NOW} />);
    expect(screen.getByTestId('prediction-ledger-loading')).toBeInTheDocument();
    expect(container.querySelectorAll('.animate-pulse').length).toBeGreaterThanOrEqual(3);
  });

  it('renders the empty state when there are no records', () => {
    render(<PredictionLedger records={[]} nowSeconds={NOW} />);
    expect(screen.getByText('No predictions yet.')).toBeInTheDocument();
  });

  it('submits the add form with the entered values', () => {
    const onAdd = vi.fn();
    render(<PredictionLedger records={[]} nowSeconds={NOW} onAdd={onAdd} />);
    fireEvent.change(screen.getByLabelText('Prediction'), {
      target: { value: 'It will rain tomorrow' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Add prediction' }));
    expect(onAdd).toHaveBeenCalledTimes(1);
    expect(onAdd.mock.calls[0][0]).toMatchObject({
      statement: 'It will rain tomorrow',
      confidencePercent: 70,
      resolveBy: null,
      category: null,
    });
  });

  it('does not submit a blank statement', () => {
    const onAdd = vi.fn();
    render(<PredictionLedger records={[]} nowSeconds={NOW} onAdd={onAdd} />);
    expect(screen.getByRole('button', { name: 'Add prediction' })).toBeDisabled();
    fireEvent.click(screen.getByRole('button', { name: 'Add prediction' }));
    expect(onAdd).not.toHaveBeenCalled();
  });

  it('renders Brier headline + calibration curve for resolved predictions', () => {
    render(<PredictionLedger records={resolvedFixture} nowSeconds={NOW} />);
    expect(screen.getByText('Brier score')).toBeInTheDocument();
    expect(screen.getByText('Calibration curve')).toBeInTheDocument();
    // Two resolved predictions in distinct bins -> two occupied bin rows.
    expect(screen.getByText('Resolved')).toBeInTheDocument();
  });

  it('fires onResolve with the id and chosen outcome from the open list', () => {
    const onResolve = vi.fn();
    render(
      <PredictionLedger records={[rec({ id: 'o' })]} nowSeconds={NOW} onResolve={onResolve} />
    );
    fireEvent.click(screen.getByRole('button', { name: /as Came true/ }));
    expect(onResolve).toHaveBeenLastCalledWith('o', 'true' satisfies PredictionOutcome);
    fireEvent.click(screen.getByRole('button', { name: /as Did not/ }));
    expect(onResolve).toHaveBeenLastCalledWith('o', 'false' satisfies PredictionOutcome);
  });

  it('shows an Overdue badge for an open prediction past its resolveBy', () => {
    render(<PredictionLedger records={[rec({ id: 'o', resolveBy: NOW - 10 })]} nowSeconds={NOW} />);
    expect(screen.getByText('Overdue')).toBeInTheDocument();
  });

  it('does not show Overdue when the prediction is not past due', () => {
    render(<PredictionLedger records={[rec({ id: 'o', resolveBy: NOW + 10 })]} nowSeconds={NOW} />);
    expect(screen.queryByText('Overdue')).not.toBeInTheDocument();
  });

  it('fires onDelete from the resolved list', () => {
    const onDelete = vi.fn();
    render(
      <PredictionLedger
        records={[rec({ id: 'r', outcome: 'true', resolvedAt: NOW })]}
        nowSeconds={NOW}
        onDelete={onDelete}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: /Delete prediction/ }));
    expect(onDelete).toHaveBeenCalledWith('r');
  });
});
