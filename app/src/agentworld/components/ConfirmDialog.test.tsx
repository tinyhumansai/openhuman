/**
 * Tests for ConfirmDialog — the in-app confirmation modal that replaces native
 * window.confirm for Agent World destructive actions (#4197). Covers rendering
 * of title/message, confirm/cancel callbacks, and the busy state.
 */
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, test, vi } from 'vitest';

import ConfirmDialog from './ConfirmDialog';

function baseProps() {
  return {
    title: 'Delete post',
    message: "Delete this post? This can't be undone.",
    onConfirm: vi.fn(),
    onCancel: vi.fn(),
  };
}

describe('ConfirmDialog', () => {
  test('renders the title and message', () => {
    render(<ConfirmDialog {...baseProps()} />);
    expect(screen.getByText('Delete post')).toBeInTheDocument();
    expect(screen.getByTestId('confirm-dialog-message')).toHaveTextContent(
      "Delete this post? This can't be undone."
    );
  });

  test('calls onConfirm when the confirm button is clicked', async () => {
    const props = baseProps();
    render(<ConfirmDialog {...props} />);
    await userEvent.click(screen.getByTestId('confirm-dialog-confirm'));
    expect(props.onConfirm).toHaveBeenCalledTimes(1);
    expect(props.onCancel).not.toHaveBeenCalled();
  });

  test('calls onCancel when the cancel button is clicked', async () => {
    const props = baseProps();
    render(<ConfirmDialog {...props} cancelLabel="Cancel" />);
    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(props.onCancel).toHaveBeenCalledTimes(1);
    expect(props.onConfirm).not.toHaveBeenCalled();
  });

  test('disables the confirm button and shows busyLabel while busy', () => {
    render(<ConfirmDialog {...baseProps()} busy busyLabel="Deleting…" />);
    const confirm = screen.getByTestId('confirm-dialog-confirm');
    expect(confirm).toBeDisabled();
    expect(confirm).toHaveTextContent('Deleting…');
  });

  test('uses a custom confirm label when provided', () => {
    render(<ConfirmDialog {...baseProps()} confirmLabel="Remove" />);
    expect(screen.getByTestId('confirm-dialog-confirm')).toHaveTextContent('Remove');
  });
});
