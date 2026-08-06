import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, test, vi } from 'vitest';

import { ConfirmDialog } from './ConfirmDialog';

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

function baseProps() {
  return {
    title: 'Confirm action',
    body: 'Continue with this action?',
    onConfirm: vi.fn(),
    onCancel: vi.fn(),
  };
}

describe('ConfirmDialog', () => {
  test('renders string and React-node bodies', () => {
    const { rerender } = render(<ConfirmDialog {...baseProps()} />);
    expect(screen.getByText('Continue with this action?')).toBeInTheDocument();

    rerender(
      <ConfirmDialog
        {...baseProps()}
        body={
          <p>
            Rich <strong>body</strong>
          </p>
        }
      />
    );
    expect(screen.getByText('Rich', { exact: false })).toContainElement(screen.getByText('body'));
  });

  test('uses neutral confirmation tone by default and danger tone when destructive', () => {
    const { rerender } = render(<ConfirmDialog {...baseProps()} />);
    expect(screen.getByRole('button', { name: 'Confirm' })).toHaveClass('bg-primary-500');

    rerender(<ConfirmDialog {...baseProps()} destructive />);
    expect(screen.getByRole('button', { name: 'Confirm' })).toHaveClass('bg-coral-500');
  });

  test('calls confirmation and cancellation actions', async () => {
    const props = baseProps();
    const user = userEvent.setup();
    render(<ConfirmDialog {...props} />);

    expect(screen.getByTestId('confirm-dialog-confirm')).toBe(
      screen.getByRole('button', { name: 'Confirm' })
    );
    await user.click(screen.getByRole('button', { name: 'Confirm' }));
    await user.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(props.onConfirm).toHaveBeenCalledTimes(1);
    expect(props.onCancel).toHaveBeenCalledTimes(1);
  });

  test('shows the busy label, disables actions, and suppresses every dismissal path', () => {
    const props = baseProps();
    render(<ConfirmDialog {...props} busy />);

    expect(screen.getByRole('button', { name: 'Working…' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeDisabled();
    expect(screen.queryByRole('button', { name: 'common.close' })).not.toBeInTheDocument();

    fireEvent.keyDown(document, { key: 'Escape' });
    fireEvent.pointerDown(screen.getByRole('dialog').parentElement!);
    expect(props.onCancel).not.toHaveBeenCalled();
  });

  test('confirmDisabled disables only confirmation and preserves dismissal policy', async () => {
    const props = baseProps();
    const user = userEvent.setup();
    render(<ConfirmDialog {...props} confirmDisabled />);

    expect(screen.getByRole('button', { name: 'Confirm' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'common.close' })).toBeInTheDocument();

    fireEvent.keyDown(document, { key: 'Escape' });
    expect(props.onCancel).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(props.onCancel).toHaveBeenCalledTimes(2);
  });

  test('restores focus after dismissal unmounts the dialog', async () => {
    const trigger = document.createElement('button');
    document.body.appendChild(trigger);
    trigger.focus();
    const props = baseProps();
    const user = userEvent.setup();
    const { unmount } = render(<ConfirmDialog {...props} />);

    expect(screen.getByRole('dialog')).toHaveFocus();
    await user.click(screen.getByRole('button', { name: 'Cancel' }));
    unmount();

    expect(trigger).toHaveFocus();
    trigger.remove();
  });
});
