/**
 * Tests for McpConnectionHealthToolbar — aggregate status counts +
 * Retry All / Disconnect All bulk-action surface with confirmation
 * dialog.
 */
import { act, fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import McpConnectionHealthToolbar from './McpConnectionHealthToolbar';
import type { ConnStatus, ServerStatus } from './types';

const statusFor = (server_id: string, status: ServerStatus): ConnStatus => ({
  server_id,
  qualified_name: `acme/${server_id}`,
  display_name: server_id,
  status,
  tool_count: status === 'connected' ? 3 : 0,
});

describe('McpConnectionHealthToolbar', () => {
  it('renders nothing when statuses array is empty', () => {
    const { container } = render(
      <McpConnectionHealthToolbar
        statuses={[]}
        onReconnect={async () => {}}
        onDisconnect={async () => {}}
      />
    );
    expect(container.firstChild).toBeNull();
  });

  it('always shows connected + disconnected counts (even when zero)', () => {
    render(
      <McpConnectionHealthToolbar
        statuses={[statusFor('a', 'disconnected')]}
        onReconnect={async () => {}}
        onDisconnect={async () => {}}
      />
    );
    expect(screen.getByText('0 connected')).toBeInTheDocument();
    expect(screen.getByText('1 idle')).toBeInTheDocument();
  });

  it('only shows connecting count when there are connecting servers', () => {
    const { rerender } = render(
      <McpConnectionHealthToolbar
        statuses={[statusFor('a', 'connected'), statusFor('b', 'error')]}
        onReconnect={async () => {}}
        onDisconnect={async () => {}}
      />
    );
    expect(screen.queryByText(/connecting/)).not.toBeInTheDocument();
    rerender(
      <McpConnectionHealthToolbar
        statuses={[statusFor('a', 'connected'), statusFor('b', 'connecting')]}
        onReconnect={async () => {}}
        onDisconnect={async () => {}}
      />
    );
    expect(screen.getByText('1 connecting')).toBeInTheDocument();
  });

  it('only shows error count when there are errored servers', () => {
    const { rerender } = render(
      <McpConnectionHealthToolbar
        statuses={[statusFor('a', 'connected')]}
        onReconnect={async () => {}}
        onDisconnect={async () => {}}
      />
    );
    expect(screen.queryByText(/error/)).not.toBeInTheDocument();
    rerender(
      <McpConnectionHealthToolbar
        statuses={[statusFor('a', 'connected'), statusFor('b', 'error')]}
        onReconnect={async () => {}}
        onDisconnect={async () => {}}
      />
    );
    expect(screen.getByText('1 error')).toBeInTheDocument();
  });

  it('aggregates counts correctly across a mixed status set', () => {
    render(
      <McpConnectionHealthToolbar
        statuses={[
          statusFor('a', 'connected'),
          statusFor('b', 'connected'),
          statusFor('c', 'error'),
          statusFor('d', 'connecting'),
          statusFor('e', 'disconnected'),
          statusFor('f', 'disconnected'),
        ]}
        onReconnect={async () => {}}
        onDisconnect={async () => {}}
      />
    );
    expect(screen.getByText('2 connected')).toBeInTheDocument();
    expect(screen.getByText('1 connecting')).toBeInTheDocument();
    expect(screen.getByText('1 error')).toBeInTheDocument();
    expect(screen.getByText('2 idle')).toBeInTheDocument();
  });

  it('hides "Retry all" button when there are no errors', () => {
    render(
      <McpConnectionHealthToolbar
        statuses={[statusFor('a', 'connected'), statusFor('b', 'disconnected')]}
        onReconnect={async () => {}}
        onDisconnect={async () => {}}
      />
    );
    expect(screen.queryByRole('button', { name: /Retry all/i })).not.toBeInTheDocument();
  });

  it('hides "Disconnect all" button when nothing is connected', () => {
    render(
      <McpConnectionHealthToolbar
        statuses={[statusFor('a', 'error'), statusFor('b', 'disconnected')]}
        onReconnect={async () => {}}
        onDisconnect={async () => {}}
      />
    );
    expect(screen.queryByRole('button', { name: /Disconnect all/i })).not.toBeInTheDocument();
  });

  it('shows "Retry all (N)" button with the correct error count when errors exist', () => {
    render(
      <McpConnectionHealthToolbar
        statuses={[statusFor('a', 'error'), statusFor('b', 'error'), statusFor('c', 'connected')]}
        onReconnect={async () => {}}
        onDisconnect={async () => {}}
      />
    );
    expect(
      screen.getByRole('button', { name: 'Retry all 2 errored MCP servers' })
    ).toBeInTheDocument();
    expect(screen.getByText('Retry all (2)')).toBeInTheDocument();
  });

  it('calls onReconnect with the errored server IDs when "Retry all" is clicked', async () => {
    const onReconnect = vi.fn().mockResolvedValue(undefined);
    render(
      <McpConnectionHealthToolbar
        statuses={[
          statusFor('srv-1', 'error'),
          statusFor('srv-2', 'connected'),
          statusFor('srv-3', 'error'),
        ]}
        onReconnect={onReconnect}
        onDisconnect={async () => {}}
      />
    );
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /Retry all/i }));
    });
    expect(onReconnect).toHaveBeenCalledTimes(1);
    expect(onReconnect).toHaveBeenCalledWith(['srv-1', 'srv-3']);
  });

  it('does NOT call onDisconnect directly — opens confirm dialog first', () => {
    const onDisconnect = vi.fn();
    render(
      <McpConnectionHealthToolbar
        statuses={[statusFor('a', 'connected')]}
        onReconnect={async () => {}}
        onDisconnect={onDisconnect}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: /Disconnect all/i }));
    expect(onDisconnect).not.toHaveBeenCalled();
    // Confirm dialog appears with accessible structure
    const dialog = screen.getByRole('dialog');
    expect(dialog).toHaveAttribute('aria-modal', 'true');
    expect(screen.getByText('Disconnect all MCP servers?')).toBeInTheDocument();
  });

  it('cancel in the dialog closes it without calling onDisconnect', () => {
    const onDisconnect = vi.fn();
    render(
      <McpConnectionHealthToolbar
        statuses={[statusFor('a', 'connected')]}
        onReconnect={async () => {}}
        onDisconnect={onDisconnect}
      />
    );
    fireEvent.click(
      screen.getByRole('button', { name: /Disconnect all \d+ connected MCP servers/i })
    );
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(onDisconnect).not.toHaveBeenCalled();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('Escape closes the confirm dialog without calling onDisconnect', () => {
    const onDisconnect = vi.fn();
    render(
      <McpConnectionHealthToolbar
        statuses={[statusFor('a', 'connected')]}
        onReconnect={async () => {}}
        onDisconnect={onDisconnect}
      />
    );
    fireEvent.click(
      screen.getByRole('button', { name: /Disconnect all \d+ connected MCP servers/i })
    );
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    act(() => {
      fireEvent.keyDown(document, { key: 'Escape' });
    });
    expect(onDisconnect).not.toHaveBeenCalled();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('confirm in the dialog fires onDisconnect with connected IDs and closes the dialog', async () => {
    const onDisconnect = vi.fn().mockResolvedValue(undefined);
    render(
      <McpConnectionHealthToolbar
        statuses={[
          statusFor('srv-1', 'connected'),
          statusFor('srv-2', 'error'),
          statusFor('srv-3', 'connected'),
        ]}
        onReconnect={async () => {}}
        onDisconnect={onDisconnect}
      />
    );
    fireEvent.click(
      screen.getByRole('button', { name: /Disconnect all \d+ connected MCP servers/i })
    );
    // Confirm button inside dialog
    const dialogConfirm = screen.getAllByRole('button', { name: 'Disconnect all' })[0];
    await act(async () => {
      fireEvent.click(dialogConfirm);
    });
    expect(onDisconnect).toHaveBeenCalledTimes(1);
    expect(onDisconnect).toHaveBeenCalledWith(['srv-1', 'srv-3']);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('disables both action buttons while a bulk operation is pending', async () => {
    let resolveOp: (() => void) | undefined;
    const onReconnect = vi.fn(
      () =>
        new Promise<void>(res => {
          resolveOp = res;
        })
    );
    render(
      <McpConnectionHealthToolbar
        statuses={[statusFor('a', 'error'), statusFor('b', 'connected')]}
        onReconnect={onReconnect}
        onDisconnect={async () => {}}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: /Retry all/i }));
    // While the promise is pending, both buttons should be disabled.
    expect(screen.getByRole('button', { name: /Retry all/i })).toBeDisabled();
    expect(
      screen.getByRole('button', { name: /Disconnect all \d+ connected MCP servers/i })
    ).toBeDisabled();
    // Resolve and re-render — buttons re-enable.
    await act(async () => {
      resolveOp?.();
    });
    expect(screen.getByRole('button', { name: /Retry all/i })).not.toBeDisabled();
  });

  it('surfaces a thrown error from onReconnect via role="alert"', async () => {
    const onReconnect = vi.fn().mockRejectedValue(new Error('upstream RPC died'));
    render(
      <McpConnectionHealthToolbar
        statuses={[statusFor('a', 'error')]}
        onReconnect={onReconnect}
        onDisconnect={async () => {}}
      />
    );
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /Retry all/i }));
    });
    const alert = screen.getByRole('alert');
    expect(alert).toHaveTextContent('upstream RPC died');
  });

  it('falls back to a generic error message when the thrown value is not an Error instance', async () => {
    const onReconnect = vi.fn().mockRejectedValue('not-an-error-object');
    render(
      <McpConnectionHealthToolbar
        statuses={[statusFor('a', 'error')]}
        onReconnect={onReconnect}
        onDisconnect={async () => {}}
      />
    );
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /Retry all/i }));
    });
    expect(screen.getByRole('alert')).toHaveTextContent('Bulk operation failed. See logs.');
  });

  it('the summary region is a polite live region with an accessible label', () => {
    render(
      <McpConnectionHealthToolbar
        statuses={[statusFor('a', 'connected')]}
        onReconnect={async () => {}}
        onDisconnect={async () => {}}
      />
    );
    const status = screen.getByRole('status', { name: 'MCP connection health summary' });
    expect(status).toHaveAttribute('aria-live', 'polite');
  });

  it('confirm dialog body interpolates the connected count', () => {
    render(
      <McpConnectionHealthToolbar
        statuses={[
          statusFor('a', 'connected'),
          statusFor('b', 'connected'),
          statusFor('c', 'connected'),
        ]}
        onReconnect={async () => {}}
        onDisconnect={async () => {}}
      />
    );
    fireEvent.click(
      screen.getByRole('button', { name: /Disconnect all \d+ connected MCP servers/i })
    );
    expect(
      screen.getByText(/This will disconnect 3 currently-connected MCP servers/)
    ).toBeInTheDocument();
  });
});
