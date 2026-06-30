/**
 * Tests for AmountCommitDialog — the amount-entry dialog for x402 bid/offer
 * commitments. The dialog now accepts a HUMAN decimal amount and emits BASE
 * units (× 10^decimals) to the existing onSubmit contract. Generic placeholders
 * only.
 */
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, test, vi } from 'vitest';

import AmountCommitDialog, { parseHumanAmount } from './AmountCommitDialog';

function baseProps() {
  return {
    title: 'Bid on @handle',
    asset: 'USDC',
    decimals: 6,
    submitLabel: 'Continue',
    onSubmit: vi.fn(),
    onCancel: vi.fn(),
  };
}

describe('parseHumanAmount', () => {
  test('scales a human decimal amount to base units', () => {
    expect(parseHumanAmount('1.5', 6)).toMatchObject({ base: '1500000', valid: true });
    expect(parseHumanAmount('1', 6)).toMatchObject({ base: '1000000', valid: true });
    expect(parseHumanAmount('0.000001', 6)).toMatchObject({ base: '1', valid: true });
    expect(parseHumanAmount('35', 6)).toMatchObject({ base: '35000000', valid: true });
  });

  test('empty / partial input is not valid but carries no error', () => {
    expect(parseHumanAmount('', 6)).toMatchObject({ valid: false, errorKey: null });
    expect(parseHumanAmount('.', 6)).toMatchObject({ valid: false, errorKey: null });
  });

  test('rejects more decimal places than the asset supports', () => {
    expect(parseHumanAmount('1.1234567', 6)).toMatchObject({
      valid: false,
      errorKey: 'agentWorld.trading.amountTooManyDecimals',
    });
  });

  test('rejects zero and non-positive amounts', () => {
    expect(parseHumanAmount('0', 6)).toMatchObject({
      valid: false,
      errorKey: 'agentWorld.trading.amountMustBePositive',
    });
    expect(parseHumanAmount('0.0', 6)).toMatchObject({
      valid: false,
      errorKey: 'agentWorld.trading.amountMustBePositive',
    });
  });
});

describe('AmountCommitDialog', () => {
  test('submit is disabled until a valid amount is entered', async () => {
    render(<AmountCommitDialog {...baseProps()} />);
    expect(screen.getByTestId('commit-submit')).toBeDisabled();
    await userEvent.type(screen.getByTestId('commit-amount-input'), '5');
    expect(screen.getByTestId('commit-submit')).toBeEnabled();
  });

  test('a human decimal amount is converted to base units on submit', async () => {
    const props = baseProps();
    render(<AmountCommitDialog {...props} />);
    const input = screen.getByTestId('commit-amount-input') as HTMLInputElement;
    await userEvent.type(input, '1.5');
    expect(input.value).toBe('1.5');
    await userEvent.click(screen.getByTestId('commit-submit'));
    expect(props.onSubmit).toHaveBeenCalledWith('1500000');
  });

  test('input strips letters and collapses to a single decimal point', async () => {
    render(<AmountCommitDialog {...baseProps()} />);
    const input = screen.getByTestId('commit-amount-input') as HTMLInputElement;
    await userEvent.type(input, '1a2.3.4b');
    // letters dropped, only the first dot kept: "12.34"
    expect(input.value).toBe('12.34');
  });

  test('shows an error and blocks submit when too many decimals are entered', async () => {
    const props = baseProps();
    render(<AmountCommitDialog {...props} />);
    await userEvent.type(screen.getByTestId('commit-amount-input'), '1.1234567');
    expect(screen.getByTestId('commit-amount-error')).toBeInTheDocument();
    expect(screen.getByTestId('commit-submit')).toBeDisabled();
  });

  test('cancel calls onCancel', async () => {
    const props = baseProps();
    render(<AmountCommitDialog {...props} />);
    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(props.onCancel).toHaveBeenCalledTimes(1);
  });

  test('busy disables the input and both actions and shows the busy label', () => {
    render(<AmountCommitDialog {...baseProps()} busy busyLabel="Submitting…" />);
    expect(screen.getByTestId('commit-amount-input')).toBeDisabled();
    const submit = screen.getByTestId('commit-submit');
    expect(submit).toHaveTextContent('Submitting…');
    expect(submit).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeDisabled();
  });

  test('Escape while busy is a no-op (does not cancel mid-submit)', async () => {
    const props = baseProps();
    render(<AmountCommitDialog {...props} busy busyLabel="Submitting…" />);
    await userEvent.keyboard('{Escape}');
    expect(props.onCancel).not.toHaveBeenCalled();
  });
});
