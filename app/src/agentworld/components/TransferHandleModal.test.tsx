/**
 * Tests for TransferHandleModal (GH-4929) — the confirm + execute dialog for a
 * Tiny Place handle transfer. A transfer is destructive/irreversible, so these
 * assert the wiring to `apiClient.registry.transfer`, that confirm is gated on a
 * recipient, and that the flow fails CLOSED (error keeps the dialog open and
 * never reports success).
 *
 * All handles/recipients are generic placeholders, never real identities.
 */
import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import { apiClient } from '../AgentWorldShell';
import TransferHandleModal from './TransferHandleModal';

vi.mock('../AgentWorldShell', () => ({ apiClient: { registry: { transfer: vi.fn() } } }));

const transfer = vi.mocked(apiClient.registry.transfer);

beforeEach(() => {
  vi.clearAllMocks();
});

function setup() {
  const onClose = vi.fn();
  const onTransferred = vi.fn();
  renderWithProviders(
    <TransferHandleModal handle="alpha" onClose={onClose} onTransferred={onTransferred} />
  );
  return { onClose, onTransferred };
}

describe('TransferHandleModal', () => {
  test('shows the handle + irreversible warning and gates confirm on a recipient', () => {
    setup();
    expect(screen.getByTestId('transfer-handle-modal')).toBeInTheDocument();
    expect(screen.getByText('@alpha')).toBeInTheDocument();
    expect(screen.getByText(/permanent and cannot be undone/i)).toBeInTheDocument();
    // Confirm is disabled until a recipient is entered.
    expect(screen.getByTestId('transfer-handle-confirm')).toBeDisabled();
  });

  test('keeps confirm disabled until the exact handle is re-typed', async () => {
    const user = userEvent.setup();
    setup();
    const confirmBtn = screen.getByTestId('transfer-handle-confirm');
    // A recipient alone is not enough for a destructive action.
    await user.type(screen.getByPlaceholderText(/Recipient @handle/i), 'bravo');
    expect(confirmBtn).toBeDisabled();
    // A wrong handle keeps it disabled.
    await user.type(screen.getByTestId('transfer-handle-confirm-input'), 'wrong');
    expect(confirmBtn).toBeDisabled();
    // The exact handle (case- and @-insensitive) enables it.
    await user.clear(screen.getByTestId('transfer-handle-confirm-input'));
    await user.type(screen.getByTestId('transfer-handle-confirm-input'), '@ALPHA');
    expect(confirmBtn).toBeEnabled();
  });

  test('confirming transfers to the resolved recipient, then closes on success', async () => {
    const user = userEvent.setup();
    transfer.mockResolvedValueOnce({ identity: { username: 'alpha' } as never });
    const { onClose, onTransferred } = setup();

    await user.type(screen.getByPlaceholderText(/Recipient @handle/i), '@bravo');
    // The irreversible action is gated behind re-typing the handle.
    await user.type(screen.getByTestId('transfer-handle-confirm-input'), '@alpha');
    await user.click(screen.getByTestId('transfer-handle-confirm'));

    // Leading @ is stripped before the RPC; handle passed through verbatim.
    await waitFor(() => expect(transfer).toHaveBeenCalledWith('alpha', 'bravo'));
    await waitFor(() => expect(onTransferred).toHaveBeenCalledTimes(1));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId('transfer-handle-error')).not.toBeInTheDocument();
  });

  test('fails closed: on error it shows the message and does not report success', async () => {
    const user = userEvent.setup();
    transfer.mockRejectedValueOnce(new Error('recipient handle is not registered on tiny.place'));
    const { onClose, onTransferred } = setup();

    await user.type(screen.getByPlaceholderText(/Recipient @handle/i), 'bravo');
    await user.type(screen.getByTestId('transfer-handle-confirm-input'), 'alpha');
    await user.click(screen.getByTestId('transfer-handle-confirm'));

    await waitFor(() =>
      expect(screen.getByTestId('transfer-handle-error')).toHaveTextContent(/not registered/i)
    );
    // Fail closed: no success callbacks, dialog stays open.
    expect(onTransferred).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByTestId('transfer-handle-modal')).toBeInTheDocument();
  });
});
