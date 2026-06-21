/**
 * Tests for AmountCommitDialog — the amount-entry dialog for x402 bid/offer
 * commitments. Generic placeholders only.
 */
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, test, vi } from 'vitest';

import AmountCommitDialog from './AmountCommitDialog';

function baseProps() {
  return {
    title: 'Bid on @handle',
    asset: 'USDC',
    submitLabel: 'Place bid',
    onSubmit: vi.fn(),
    onCancel: vi.fn(),
  };
}

describe('AmountCommitDialog', () => {
  test('submit is disabled until a numeric amount is entered', async () => {
    render(<AmountCommitDialog {...baseProps()} />);
    expect(screen.getByTestId('commit-submit')).toBeDisabled();
    await userEvent.type(screen.getByTestId('commit-amount-input'), '500');
    expect(screen.getByTestId('commit-submit')).toBeEnabled();
  });

  test('input strips non-digits and submit reports the sanitized amount', async () => {
    const props = baseProps();
    render(<AmountCommitDialog {...props} />);
    const input = screen.getByTestId('commit-amount-input') as HTMLInputElement;
    await userEvent.type(input, '1a2b3');
    expect(input.value).toBe('123');
    await userEvent.click(screen.getByTestId('commit-submit'));
    expect(props.onSubmit).toHaveBeenCalledWith('123');
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
