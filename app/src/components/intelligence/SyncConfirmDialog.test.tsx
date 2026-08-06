import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { callCoreRpc } from '../../services/coreRpcClient';
import SyncConfirmDialog from './SyncConfirmDialog';

vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

vi.mock('../../lib/i18n/I18nContext', () => ({
  useT: () => ({
    t: (key: string) =>
      ({
        'syncConfirm.title': 'Confirm sync',
        'syncConfirm.message':
          'This sync will process ~{items} items (~{tokens} tokens, estimated ${cost}).',
        'syncConfirm.budgetNote': 'Budget limit: ${max}',
        'syncConfirm.proceed': 'Proceed',
        'syncConfirm.cancel': 'Cancel',
        'syncConfirm.estimating': 'Estimating cost...',
        'common.close': 'Close',
      })[key] ?? key,
  }),
}));

const estimate = {
  item_count: 12,
  estimated_tokens: 2_400,
  estimated_cost_usd: 0.0123,
  budget_max_cost_usd: 1,
  budget_max_tokens: 10_000,
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe('SyncConfirmDialog', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  test('is a named dialog with loading, error, and ready body states', async () => {
    const request = deferred<{ result: typeof estimate }>();
    vi.mocked(callCoreRpc).mockReturnValue(request.promise);
    const onConfirm = vi.fn();

    render(<SyncConfirmDialog sourceId="source-1" onConfirm={onConfirm} onCancel={vi.fn()} />);

    expect(screen.getByRole('dialog', { name: 'Confirm sync' })).toBeInTheDocument();
    expect(screen.getByText('Estimating cost...')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Proceed' })).toBeDisabled();
    fireEvent.click(screen.getByRole('button', { name: 'Proceed' }));
    expect(onConfirm).not.toHaveBeenCalled();

    request.resolve({ result: estimate });
    expect(
      await screen.findByText('This sync will process ~12 items (~2k tokens, estimated $0.0123).')
    ).toBeInTheDocument();
    expect(screen.getByText('Budget limit: $1.00')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Proceed' })).toBeEnabled();
  });

  test('shows estimate errors and keeps confirmation disabled', async () => {
    vi.mocked(callCoreRpc).mockRejectedValue(new Error('Estimate unavailable'));

    render(<SyncConfirmDialog sourceId="source-1" onConfirm={vi.fn()} onCancel={vi.fn()} />);

    expect(await screen.findByText('Estimate unavailable')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Proceed' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeEnabled();
  });

  test('cancels from Escape and backdrop and restores focus after dismissal', async () => {
    vi.mocked(callCoreRpc).mockResolvedValue({ result: estimate });
    const trigger = document.createElement('button');
    document.body.appendChild(trigger);
    trigger.focus();
    const onCancel = vi.fn();
    const { unmount } = render(
      <SyncConfirmDialog sourceId="source-1" onConfirm={vi.fn()} onCancel={onCancel} />
    );

    expect(screen.getByRole('dialog')).toHaveFocus();
    fireEvent.keyDown(document, { key: 'Escape' });
    fireEvent.pointerDown(screen.getByRole('dialog').parentElement!);
    expect(onCancel).toHaveBeenCalledTimes(2);

    unmount();
    await waitFor(() => expect(trigger).toHaveFocus());
    trigger.remove();
  });
});
