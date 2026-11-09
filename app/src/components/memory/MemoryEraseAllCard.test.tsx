import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import MemoryEraseAllCard, { eraseErrorKey } from './MemoryEraseAllCard';

const hoisted = vi.hoisted(() => ({ erase: vi.fn(), toastAdd: vi.fn(), track: vi.fn() }));

vi.mock('../../services/api/memoryApi', async importOriginal => ({
  ...(await importOriginal<typeof import('../../services/api/memoryApi')>()),
  memoryEraseAll: (...a: unknown[]) => hoisted.erase(...a),
}));
vi.mock('../ui/Toast', () => ({ toast: { add: (...a: unknown[]) => hoisted.toastAdd(...a) } }));
vi.mock('../analytics', () => ({ trackAnalyticsEvent: (...a: unknown[]) => hoisted.track(...a) }));

beforeEach(() => {
  hoisted.erase.mockReset();
  hoisted.toastAdd.mockReset();
  hoisted.track.mockReset();
});

async function openDialog() {
  fireEvent.click(screen.getByTestId('memory-erase-open'));
  return screen.findByTestId('memory-erase-dialog');
}

describe('MemoryEraseAllCard', () => {
  it('renders the destructive control', () => {
    renderWithProviders(<MemoryEraseAllCard />);
    expect(screen.getByTestId('memory-erase-card')).toHaveTextContent('Erase memory');
    const open = screen.getByTestId('memory-erase-open');
    expect(open).toHaveTextContent('Erase all memory');
    expect(open).toHaveAttribute('data-analytics-id', 'memory-erase-all-open');
    expect(open).not.toBeDisabled();
  });

  it('is disabled when memory is off', () => {
    renderWithProviders(<MemoryEraseAllCard disabled />);
    expect(screen.getByTestId('memory-erase-open')).toBeDisabled();
  });

  it('explains the erase and gates it behind the acknowledgement', async () => {
    renderWithProviders(<MemoryEraseAllCard />);
    const dialog = await openDialog();
    expect(dialog).toHaveTextContent('every connected source, past conversation');
    expect(dialog).toHaveTextContent('This cannot be undone.');
    const confirm = screen.getByTestId('memory-erase-confirm');
    expect(confirm).toBeDisabled();
    fireEvent.click(confirm);
    expect(hoisted.erase).not.toHaveBeenCalled();

    fireEvent.click(screen.getByTestId('memory-erase-ack'));
    expect(confirm).not.toBeDisabled();
  });

  it('erases with the confirm interlock, toasts, tracks and refreshes', async () => {
    hoisted.erase.mockResolvedValue({ erased_scopes: 4 });
    const onErased = vi.fn();
    renderWithProviders(<MemoryEraseAllCard onErased={onErased} />);
    await openDialog();
    fireEvent.click(screen.getByTestId('memory-erase-ack'));
    fireEvent.click(screen.getByTestId('memory-erase-confirm'));

    await waitFor(() => expect(hoisted.erase).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(onErased).toHaveBeenCalledTimes(1));
    expect(hoisted.toastAdd).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'success', title: 'All memory erased' })
    );
    expect(hoisted.track).toHaveBeenCalledWith('memory_erased_all');
    await waitFor(() =>
      expect(screen.queryByTestId('memory-erase-dialog')).not.toBeInTheDocument()
    );
  });

  it('does not report a failed refresh callback as a failed erase', async () => {
    hoisted.erase.mockResolvedValue({ erased_scopes: 1 });
    const onErased = vi.fn(() => {
      throw new Error('refresh boom');
    });
    renderWithProviders(<MemoryEraseAllCard onErased={onErased} />);
    await openDialog();
    fireEvent.click(screen.getByTestId('memory-erase-ack'));
    fireEvent.click(screen.getByTestId('memory-erase-confirm'));

    await waitFor(() => expect(onErased).toHaveBeenCalledTimes(1));
    expect(screen.queryByTestId('memory-erase-error')).not.toBeInTheDocument();
    await waitFor(() =>
      expect(screen.queryByTestId('memory-erase-dialog')).not.toBeInTheDocument()
    );
  });

  it('keeps the erase a success when the success reporting throws', async () => {
    hoisted.erase.mockResolvedValue({ erased_scopes: 1 });
    hoisted.track.mockImplementationOnce(() => {
      throw new Error('analytics boom');
    });
    const onErased = vi.fn();
    renderWithProviders(<MemoryEraseAllCard onErased={onErased} />);
    await openDialog();
    fireEvent.click(screen.getByTestId('memory-erase-ack'));
    fireEvent.click(screen.getByTestId('memory-erase-confirm'));

    await waitFor(() => expect(onErased).toHaveBeenCalledTimes(1));
    expect(screen.queryByTestId('memory-erase-error')).not.toBeInTheDocument();
  });

  it('contains a rejected async refresh callback', async () => {
    hoisted.erase.mockResolvedValue({ erased_scopes: 1 });
    const onErased = vi.fn(() => Promise.reject(new Error('refresh boom')));
    renderWithProviders(<MemoryEraseAllCard onErased={onErased} />);
    await openDialog();
    fireEvent.click(screen.getByTestId('memory-erase-ack'));
    fireEvent.click(screen.getByTestId('memory-erase-confirm'));

    await waitFor(() => expect(onErased).toHaveBeenCalledTimes(1));
    expect(screen.queryByTestId('memory-erase-error')).not.toBeInTheDocument();
  });

  it('translates an unauthorized failure without a success toast, event or refresh', async () => {
    hoisted.erase.mockRejectedValue(
      Object.assign(new Error('UNAUTHORIZED: secret'), { data: { code: 'UNAUTHORIZED' } })
    );
    const onErased = vi.fn();
    renderWithProviders(<MemoryEraseAllCard onErased={onErased} />);
    await openDialog();
    fireEvent.click(screen.getByTestId('memory-erase-ack'));
    fireEvent.click(screen.getByTestId('memory-erase-confirm'));

    const error = await screen.findByTestId('memory-erase-error');
    expect(error).not.toHaveTextContent('secret');
    expect(onErased).not.toHaveBeenCalled();
    expect(hoisted.toastAdd).not.toHaveBeenCalled();
    expect(hoisted.track).not.toHaveBeenCalled();
  });

  it('shows a translated error and never the raw server text', async () => {
    hoisted.erase.mockRejectedValue(
      Object.assign(new Error('UNSUPPORTED: DELETE /memory returned 404 secret-detail'), {
        data: { code: 'UNSUPPORTED' },
      })
    );
    const onErased = vi.fn();
    renderWithProviders(<MemoryEraseAllCard onErased={onErased} />);
    await openDialog();
    fireEvent.click(screen.getByTestId('memory-erase-ack'));
    fireEvent.click(screen.getByTestId('memory-erase-confirm'));

    const error = await screen.findByTestId('memory-erase-error');
    expect(error).toHaveTextContent("Your memory service can't erase all memory yet.");
    expect(error).not.toHaveTextContent('secret-detail');
    expect(onErased).not.toHaveBeenCalled();
    expect(hoisted.toastAdd).not.toHaveBeenCalled();
    expect(hoisted.track).not.toHaveBeenCalled();
    // The dialog stays open so the user sees what happened.
    expect(screen.getByTestId('memory-erase-dialog')).toBeInTheDocument();
  });

  it('falls back to a generic message for an unknown failure', async () => {
    hoisted.erase.mockRejectedValue(new Error('boom: internal stack'));
    renderWithProviders(<MemoryEraseAllCard />);
    await openDialog();
    fireEvent.click(screen.getByTestId('memory-erase-ack'));
    fireEvent.click(screen.getByTestId('memory-erase-confirm'));
    const error = await screen.findByTestId('memory-erase-error');
    expect(error).toHaveTextContent("Couldn't erase your memory. Please try again.");
    expect(error).not.toHaveTextContent('boom');
  });

  it('maps each core error code to its own message', () => {
    const coded = (code: string) => Object.assign(new Error('x'), { data: { code } });
    expect(eraseErrorKey(coded('UNSUPPORTED'))).toBe('memoryPage.settings.eraseError.unsupported');
    expect(eraseErrorKey(coded('INSUFFICIENT_CREDITS'))).toBe(
      'memoryPage.settings.eraseError.insufficientCredits'
    );
    expect(eraseErrorKey(new Error('MEMORY_OFF: no engine bound'))).toBe(
      'memoryPage.settings.eraseError.memoryOff'
    );
    expect(eraseErrorKey(coded('UNAVAILABLE'))).toBe('memoryPage.settings.eraseError.unavailable');
    expect(eraseErrorKey(coded('UNAUTHORIZED'))).toBe(
      'memoryPage.settings.eraseError.unauthorized'
    );
    expect(eraseErrorKey(coded('PLAN_REQUIRED'))).toBe('memory.error.planRequired');
    expect(eraseErrorKey(coded('ENGINE'))).toBe('memoryPage.settings.eraseError.generic');
    expect(eraseErrorKey('nope')).toBe('memoryPage.settings.eraseError.generic');
  });
});
