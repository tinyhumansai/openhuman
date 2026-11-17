import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import MemoryImportBanner, {
  IMPORT_POLL_MS,
  MIGRATION_IDLE_POLL_MS,
  MIGRATION_POLL_MS,
} from './MemoryImportBanner';

const hoisted = vi.hoisted(() => ({
  scan: vi.fn(),
  start: vi.fn(),
  status: vi.fn(),
  retry: vi.fn(),
  mScan: vi.fn(),
  mStart: vi.fn(),
  mStatus: vi.fn(),
  mRetry: vi.fn(),
}));

vi.mock('../../services/api/memoryApi', async importOriginal => ({
  ...(await importOriginal<typeof import('../../services/api/memoryApi')>()),
  memoryImportScan: (...a: unknown[]) => hoisted.scan(...a),
  memoryImportStart: (...a: unknown[]) => hoisted.start(...a),
  memoryImportStatus: (...a: unknown[]) => hoisted.status(...a),
  memoryImportRetryFailed: (...a: unknown[]) => hoisted.retry(...a),
  memoryMigrationScan: (...a: unknown[]) => hoisted.mScan(...a),
  memoryMigrationStart: (...a: unknown[]) => hoisted.mStart(...a),
  memoryMigrationStatus: (...a: unknown[]) => hoisted.mStatus(...a),
  memoryMigrationRetry: (...a: unknown[]) => hoisted.mRetry(...a),
}));

const moving = (copied: number) => ({
  state: { phase: 'copying', copied },
  running: true,
  interrupted: false,
});
const MOVE_IDLE = { state: { phase: 'idle', copied: 0 }, running: false, interrupted: false };

const IDLE = { state: { phase: 'idle', imported: 0, total: 0 } };
const FOUND = { found: true, counts: { documents: 3, conversations: 5, learnings: 2 } };

beforeEach(() => {
  hoisted.scan.mockReset().mockResolvedValue(FOUND);
  hoisted.status.mockReset().mockResolvedValue(IDLE);
  hoisted.start.mockReset();
  hoisted.retry.mockReset();
  hoisted.mScan.mockReset().mockResolvedValue({ needed: false, shared: false });
  hoisted.mStatus.mockReset().mockResolvedValue(MOVE_IDLE);
  hoisted.mStart.mockReset().mockResolvedValue(moving(4));
  hoisted.mRetry.mockReset();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('MemoryImportBanner', () => {
  it('shows both steps disabled when there is nothing to import or move', async () => {
    hoisted.scan.mockResolvedValue({ found: false });
    renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
    const importStep = await screen.findByTestId('memory-import-idle');
    expect(importStep).toHaveTextContent('No previous memory was found on this device.');
    expect(within(importStep).getByRole('button')).toBeDisabled();
    const moveStep = screen.getByTestId('memory-migration-idle');
    expect(moveStep).toHaveTextContent('Your memory is already in your account.');
    expect(within(moveStep).getByRole('button')).toBeDisabled();
  });

  it('keeps a finished import visible, disabled', async () => {
    hoisted.scan.mockResolvedValue({ found: false });
    hoisted.status.mockResolvedValue({ state: { phase: 'done', imported: 4, total: 4 } });
    renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
    // A clean finished import shows its done state; the move step sits disabled.
    expect(await screen.findByTestId('memory-import-done')).toBeInTheDocument();
    expect(screen.getByTestId('memory-migration-idle')).toBeInTheDocument();
  });

  it('shows no "nothing to import" step when the scan or status read fails', async () => {
    hoisted.scan.mockRejectedValue(new Error('scan down'));
    const { unmount } = renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
    await waitFor(() => expect(hoisted.scan).toHaveBeenCalled());
    await screen.findByTestId('memory-import-banner');
    expect(screen.queryByTestId('memory-import-idle')).not.toBeInTheDocument();
    unmount();

    hoisted.scan.mockReset().mockResolvedValue({ found: false });
    hoisted.status.mockRejectedValue(new Error('status down'));
    renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
    await screen.findByTestId('memory-import-banner');
    expect(screen.queryByTestId('memory-import-idle')).not.toBeInTheDocument();
  });

  it('does not call a failed migration scan "already moved"', async () => {
    hoisted.mScan.mockRejectedValue(new Error('scan down'));
    renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
    await screen.findByTestId('memory-import-banner');
    await waitFor(() => expect(hoisted.mScan).toHaveBeenCalled());
    expect(screen.queryByTestId('memory-migration-idle')).not.toBeInTheDocument();
  });

  it('shows the move step disabled while the import is still on offer', async () => {
    renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
    expect(await screen.findByTestId('memory-import-counts')).toBeInTheDocument();
    expect(screen.queryByTestId('memory-import-idle')).not.toBeInTheDocument();
    expect(screen.getByTestId('memory-migration-idle')).toHaveTextContent(
      'Starts once the import finishes.'
    );
  });

  it('offers the import with the counts it found', async () => {
    renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
    expect(await screen.findByTestId('memory-import-counts')).toHaveTextContent(
      '3 documents, 5 conversations and 2 learnings'
    );
  });

  it('asks for consent naming the engine, and uploads nothing on cancel', async () => {
    renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
    fireEvent.click(await screen.findByTestId('memory-import-open'));
    expect(screen.getByTestId('memory-import-consent')).toHaveTextContent(
      'uploads it to TinyHumans'
    );
    fireEvent.click(screen.getByTestId('memory-import-cancel'));
    expect(screen.queryByTestId('memory-import-consent')).not.toBeInTheDocument();
    expect(hoisted.start).not.toHaveBeenCalled();
  });

  it('starts the import on consent and polls progress until done', async () => {
    hoisted.start.mockResolvedValue({ state: { phase: 'running', imported: 0, total: 10 } });
    renderWithProviders(<MemoryImportBanner engineLabel="CortexDB" />);
    fireEvent.click(await screen.findByTestId('memory-import-open'));

    vi.useFakeTimers({ shouldAdvanceTime: true });
    fireEvent.click(screen.getByTestId('memory-import-confirm'));
    expect(await screen.findByTestId('memory-import-running')).toHaveTextContent(
      '0 of 10 items imported'
    );
    expect(hoisted.start).toHaveBeenCalledTimes(1);

    hoisted.status.mockResolvedValue({ state: { phase: 'done', imported: 10, total: 10 } });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(IMPORT_POLL_MS + 10);
    });
    expect(await screen.findByTestId('memory-import-done')).toHaveTextContent(
      '10 of 10 items imported'
    );
    expect(screen.queryByTestId('memory-import-open')).not.toBeInTheDocument();
  });

  it('offers to retry the items a finished import could not store', async () => {
    hoisted.status.mockResolvedValue({
      state: { phase: 'done', imported: 8, total: 9, failed: 1 },
    });
    hoisted.retry.mockResolvedValue({
      state: { phase: 'running', imported: 8, total: 9, failed: 1 },
    });
    renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);

    expect(await screen.findByTestId('memory-import-failed-items')).toHaveTextContent(
      'Items that could not be imported: 1.'
    );
    fireEvent.click(screen.getByTestId('memory-import-retry-failed'));
    expect(await screen.findByTestId('memory-import-running')).toBeInTheDocument();
    expect(hoisted.retry).toHaveBeenCalledTimes(1);
    expect(hoisted.start).not.toHaveBeenCalled();
  });

  it('says why a retry stopped and keeps offering it', async () => {
    hoisted.status.mockResolvedValue({
      state: { phase: 'done', imported: 8, total: 9, failed: 1, error: 'sign in to continue' },
    });
    renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
    expect(await screen.findByTestId('memory-import-failed-items')).toHaveTextContent(
      'sign in to continue'
    );
    expect(screen.getByTestId('memory-import-retry-failed')).toBeEnabled();
  });

  it('offers no retry when nothing failed', async () => {
    hoisted.status.mockResolvedValue({
      state: { phase: 'done', imported: 9, total: 9, failed: 0 },
    });
    renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
    expect(await screen.findByTestId('memory-import-done')).toBeInTheDocument();
    expect(screen.queryByTestId('memory-import-retry-failed')).not.toBeInTheDocument();
  });

  it('shows a retry that could not start', async () => {
    hoisted.status.mockResolvedValue({
      state: { phase: 'done', imported: 8, total: 9, failed: 1 },
    });
    hoisted.retry.mockRejectedValue(new Error('no failed items to retry'));
    renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
    fireEvent.click(await screen.findByTestId('memory-import-retry-failed'));
    expect(await screen.findByTestId('memory-import-error')).toBeInTheDocument();
  });

  it('shows a failed import', async () => {
    hoisted.status.mockResolvedValue({
      state: { phase: 'error', imported: 2, total: 9, error: 'engine rejected batch' },
    });
    renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
    expect(await screen.findByTestId('memory-import-error')).toHaveTextContent(
      'engine rejected batch'
    );
  });

  it('resumes a failed import through the consent dialog', async () => {
    hoisted.status.mockResolvedValue({
      state: { phase: 'error', imported: 2, total: 9, error: 'not enough credits to import' },
    });
    hoisted.start.mockResolvedValue({ state: { phase: 'running', imported: 2, total: 9 } });
    renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);

    fireEvent.click(await screen.findByTestId('memory-import-resume'));
    expect(screen.getByTestId('memory-import-consent')).toBeInTheDocument();
    expect(hoisted.start).not.toHaveBeenCalled();

    fireEvent.click(screen.getByTestId('memory-import-confirm'));
    expect(await screen.findByTestId('memory-import-running')).toHaveTextContent(
      '2 of 9 items imported'
    );
    expect(hoisted.start).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId('memory-import-resume')).not.toBeInTheDocument();
  });

  it('offers no resume while an import is running', async () => {
    hoisted.status.mockResolvedValue({ state: { phase: 'running', imported: 1, total: 9 } });
    renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
    expect(await screen.findByTestId('memory-import-running')).toBeInTheDocument();
    expect(screen.queryByTestId('memory-import-resume')).not.toBeInTheDocument();
  });

  it('shows a start failure', async () => {
    hoisted.start.mockRejectedValue(new Error('UNAUTHORIZED: sign in again'));
    renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
    fireEvent.click(await screen.findByTestId('memory-import-open'));
    fireEvent.click(screen.getByTestId('memory-import-confirm'));
    expect(await screen.findByTestId('memory-import-error')).toHaveTextContent('sign in again');
  });

  it('does not organize while the import is still running', async () => {
    hoisted.status.mockResolvedValue({ state: { phase: 'running', imported: 1, total: 9 } });
    hoisted.mScan.mockResolvedValue({ needed: true, shared: false });
    renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
    expect(await screen.findByTestId('memory-import-running')).toBeInTheDocument();
    expect(screen.queryByTestId('memory-migration-banner')).not.toBeInTheDocument();
    expect(hoisted.mStart).not.toHaveBeenCalled();
  });

  it('shows the move the core starts once the import is done', async () => {
    hoisted.status.mockResolvedValue({ state: { phase: 'done', imported: 9, total: 9 } });
    // At mount nothing was left to move; the finished import changed that.
    hoisted.mScan
      .mockResolvedValueOnce({ needed: false, shared: false })
      .mockResolvedValue({ needed: true, shared: false });
    hoisted.mStatus.mockResolvedValueOnce(MOVE_IDLE).mockResolvedValue(moving(5));
    renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
    expect(await screen.findByTestId('memory-migration-running')).toHaveTextContent('5');
    expect(hoisted.mStart).not.toHaveBeenCalled();
  });

  it('organizes once the running import finishes', async () => {
    hoisted.status.mockResolvedValue({ state: { phase: 'running', imported: 2, total: 9 } });
    // Nothing to move until the import lands its items.
    hoisted.mScan.mockResolvedValue({ needed: false, shared: false });
    vi.useFakeTimers({ shouldAdvanceTime: true });
    renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
    expect(await screen.findByTestId('memory-import-running')).toBeInTheDocument();
    expect(screen.queryByTestId('memory-migration-banner')).not.toBeInTheDocument();

    // The import finishes and the core starts the move.
    hoisted.status.mockResolvedValue({ state: { phase: 'done', imported: 9, total: 9 } });
    hoisted.mScan.mockResolvedValue({ needed: true, shared: false });
    hoisted.mStatus.mockResolvedValue(moving(3));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(IMPORT_POLL_MS + 10);
    });
    expect(await screen.findByTestId('memory-migration-running')).toHaveTextContent('3');
    expect(screen.queryByTestId('memory-import-done')).not.toBeInTheDocument();
    expect(hoisted.mStart).not.toHaveBeenCalled();
  });

  it('keeps the retry for refused items beside the move', async () => {
    hoisted.status.mockResolvedValue({
      state: { phase: 'done', imported: 7, total: 9, failed: 2 },
    });
    hoisted.mScan.mockResolvedValue({ needed: true, shared: false });
    renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
    expect(await screen.findByTestId('memory-migration-banner')).toBeInTheDocument();
    expect(screen.getByTestId('memory-import-retry-failed')).toBeInTheDocument();
  });

  it('takes a shared tree only after the takeover is confirmed', async () => {
    hoisted.scan.mockResolvedValue({ found: false });
    hoisted.mScan.mockResolvedValue({ needed: true, shared: true });
    renderWithProviders(<MemoryImportBanner engineLabel="CortexDB" />);
    fireEvent.click(await screen.findByTestId('memory-migration-start'));
    expect(await screen.findByTestId('memory-migration-takeover')).toBeInTheDocument();
    expect(hoisted.mStart).not.toHaveBeenCalled();
    fireEvent.click(screen.getByTestId('memory-migration-takeover-confirm'));
    await waitFor(() => expect(hoisted.mStart).toHaveBeenCalledWith(true));
  });

  describe('organizing (step 2)', () => {
    beforeEach(() => {
      hoisted.scan.mockResolvedValue({ found: false });
    });

    it('migrates now without a dialog when the tree is the account’s own', async () => {
      hoisted.mScan.mockResolvedValue({ needed: true, shared: false });
      renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
      fireEvent.click(await screen.findByTestId('memory-migration-start'));
      await waitFor(() => expect(hoisted.mStart).toHaveBeenCalledWith(false));
      expect(screen.queryByTestId('memory-migration-takeover')).not.toBeInTheDocument();
      expect(await screen.findByTestId('memory-migration-running')).toHaveTextContent('4');
    });

    it('shows a failed start', async () => {
      hoisted.mScan.mockResolvedValue({ needed: true, shared: false });
      hoisted.mStart.mockRejectedValue(new Error('boom'));
      renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
      fireEvent.click(await screen.findByTestId('memory-migration-start'));
      expect(await screen.findByTestId('memory-import-error')).toBeInTheDocument();
    });

    it('cancels the takeover without starting anything', async () => {
      hoisted.mScan.mockResolvedValue({ needed: true, shared: true });
      renderWithProviders(<MemoryImportBanner engineLabel="CortexDB" />);
      fireEvent.click(await screen.findByTestId('memory-migration-start'));
      fireEvent.click(await screen.findByTestId('memory-migration-takeover-cancel'));
      expect(screen.queryByTestId('memory-migration-takeover')).not.toBeInTheDocument();
      expect(hoisted.mStart).not.toHaveBeenCalled();
    });

    it('shows why a move paused and resumes it', async () => {
      hoisted.mScan.mockResolvedValue({ needed: true, shared: false });
      hoisted.mStatus.mockResolvedValue({
        state: { phase: 'paused', copied: 3, error: 'not enough credits' },
        running: false,
        interrupted: false,
      });
      renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
      expect(await screen.findByTestId('memory-migration-paused')).toHaveTextContent(
        'not enough credits'
      );
      fireEvent.click(screen.getByTestId('memory-migration-start'));
      await waitFor(() => expect(hoisted.mStart).toHaveBeenCalledWith(false));
    });

    it('offers to try again what could not be moved', async () => {
      hoisted.mStatus.mockResolvedValue({
        state: {
          phase: 'cleaned',
          copied: 5,
          failures: [{ id: 'a', reason: 'too_large' }],
          incomplete: ['b'],
        },
        running: false,
        interrupted: false,
      });
      renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
      expect(await screen.findByTestId('memory-migration-left')).toHaveTextContent('2');
      fireEvent.click(screen.getByTestId('memory-migration-retry'));
      await waitFor(() => expect(hoisted.mRetry).toHaveBeenCalled());
      await waitFor(() => expect(hoisted.mStart).toHaveBeenCalledWith(false));
      // The scan says nothing is needed, but the retry's run stays in view.
      expect(await screen.findByTestId('memory-migration-running')).toBeInTheDocument();
    });

    it('shows a failed retry', async () => {
      hoisted.mStatus.mockResolvedValue({
        state: { phase: 'cleaned', copied: 5, failures: [{ id: 'a', reason: 'x' }] },
        running: false,
        interrupted: false,
      });
      hoisted.mRetry.mockRejectedValue(new Error('boom'));
      renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
      expect(await screen.findByTestId('memory-migration-left')).toHaveTextContent('1');
      fireEvent.click(screen.getByTestId('memory-migration-retry'));
      expect(await screen.findByTestId('memory-import-error')).toBeInTheDocument();
    });

    it('polls a running move and scans again when it ends', async () => {
      hoisted.mScan.mockResolvedValue({ needed: true, shared: false });
      hoisted.mStatus.mockResolvedValue(moving(1));
      vi.useFakeTimers({ shouldAdvanceTime: true });
      renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
      expect(await screen.findByTestId('memory-migration-running')).toHaveTextContent('1');

      hoisted.mStatus.mockResolvedValue({
        state: { phase: 'cleaned', copied: 3 },
        running: false,
        interrupted: false,
      });
      hoisted.mScan.mockResolvedValue({ needed: false, shared: false });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MIGRATION_POLL_MS + 10);
      });
      await waitFor(() =>
        expect(screen.queryByTestId('memory-migration-banner')).not.toBeInTheDocument()
      );
    });

    it('notices a move the background job starts while it is offered', async () => {
      hoisted.mScan.mockResolvedValue({ needed: true, shared: false });
      vi.useFakeTimers({ shouldAdvanceTime: true });
      renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
      expect(await screen.findByTestId('memory-migration-offer')).toBeInTheDocument();
      hoisted.mStatus.mockResolvedValue(moving(2));
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MIGRATION_IDLE_POLL_MS + 10);
      });
      expect(await screen.findByTestId('memory-migration-running')).toHaveTextContent('2');
    });

    it('shows a failed status poll', async () => {
      hoisted.mScan.mockResolvedValue({ needed: true, shared: false });
      hoisted.mStatus.mockResolvedValueOnce(moving(1)).mockRejectedValue(new Error('boom'));
      vi.useFakeTimers({ shouldAdvanceTime: true });
      renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
      expect(await screen.findByTestId('memory-migration-running')).toBeInTheDocument();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MIGRATION_POLL_MS + 10);
      });
      expect(await screen.findByTestId('memory-import-error')).toBeInTheDocument();
    });

    it('offers nothing until the move’s status is known', async () => {
      hoisted.mScan.mockResolvedValue({ needed: true, shared: false });
      hoisted.mStatus.mockRejectedValueOnce(new Error('not ready')).mockResolvedValue(MOVE_IDLE);
      vi.useFakeTimers({ shouldAdvanceTime: true });
      renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
      await waitFor(() => expect(hoisted.mStatus).toHaveBeenCalledTimes(1));
      expect(screen.queryByTestId('memory-migration-offer')).not.toBeInTheDocument();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MIGRATION_IDLE_POLL_MS + 10);
      });
      expect(await screen.findByTestId('memory-migration-offer')).toBeInTheDocument();
    });

    it('keeps both retries when import and move each left items behind', async () => {
      hoisted.status.mockResolvedValue({
        state: { phase: 'done', imported: 7, total: 9, failed: 2 },
      });
      hoisted.mStatus.mockResolvedValue({
        state: { phase: 'cleaned', copied: 7, failures: [{ id: 'a', reason: 'x' }] },
        running: false,
        interrupted: false,
      });
      renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
      expect(await screen.findByTestId('memory-migration-left')).toHaveTextContent('1');
      expect(screen.getByTestId('memory-import-retry-failed')).toBeInTheDocument();
    });

    it('offers nothing while the scan after a finished run fails', async () => {
      hoisted.mScan.mockResolvedValue({ needed: true, shared: false });
      hoisted.mStatus.mockResolvedValue(moving(1));
      vi.useFakeTimers({ shouldAdvanceTime: true });
      renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
      expect(await screen.findByTestId('memory-migration-running')).toBeInTheDocument();

      // The run ends; the scan that follows fails.
      hoisted.mStatus.mockResolvedValue({
        state: { phase: 'cleaned', copied: 3 },
        running: false,
        interrupted: false,
      });
      hoisted.mScan.mockRejectedValue(new Error('not ready'));
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MIGRATION_POLL_MS + 10);
      });
      await waitFor(() =>
        expect(screen.queryByTestId('memory-migration-banner')).not.toBeInTheDocument()
      );
      expect(screen.queryByTestId('memory-migration-offer')).not.toBeInTheDocument();
    });

    it('drops a poll answered after the user started the move', async () => {
      hoisted.mScan.mockResolvedValue({ needed: true, shared: false });
      vi.useFakeTimers({ shouldAdvanceTime: true });
      renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
      expect(await screen.findByTestId('memory-migration-offer')).toBeInTheDocument();

      // An idle poll goes out and is slow to answer.
      let answerPoll: (value: unknown) => void = () => {};
      hoisted.mStatus.mockReturnValueOnce(
        new Promise(resolve => {
          answerPoll = resolve;
        })
      );
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MIGRATION_IDLE_POLL_MS + 10);
      });
      fireEvent.click(screen.getByTestId('memory-migration-start'));
      expect(await screen.findByTestId('memory-migration-running')).toHaveTextContent('4');

      // Its stale idle answer must not take the running move off the screen.
      await act(async () => {
        answerPoll(MOVE_IDLE);
      });
      expect(screen.getByTestId('memory-migration-running')).toBeInTheDocument();
      expect(screen.queryByTestId('memory-migration-offer')).not.toBeInTheDocument();
    });

    it('drops a poll asked while the start was in flight', async () => {
      hoisted.mScan.mockResolvedValue({ needed: true, shared: false });
      vi.useFakeTimers({ shouldAdvanceTime: true });
      renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
      expect(await screen.findByTestId('memory-migration-offer')).toBeInTheDocument();

      // The start is slow to answer.
      let answerStart: (value: unknown) => void = () => {};
      hoisted.mStart.mockReturnValueOnce(
        new Promise(resolve => {
          answerStart = resolve;
        })
      );
      fireEvent.click(screen.getByTestId('memory-migration-start'));
      // Meanwhile a poll reads the state from before the move began.
      let answerPoll: (value: unknown) => void = () => {};
      hoisted.mStatus.mockReturnValueOnce(
        new Promise(resolve => {
          answerPoll = resolve;
        })
      );
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MIGRATION_IDLE_POLL_MS + 10);
      });
      await act(async () => {
        answerStart(moving(4));
      });
      expect(await screen.findByTestId('memory-migration-running')).toHaveTextContent('4');

      // That poll answers last; it must not bring the offer back.
      await act(async () => {
        answerPoll(MOVE_IDLE);
      });
      expect(screen.getByTestId('memory-migration-running')).toBeInTheDocument();
      expect(screen.queryByTestId('memory-migration-offer')).not.toBeInTheDocument();
    });

    it('drops a read still pending when the user starts the move', async () => {
      hoisted.mScan.mockResolvedValue({ needed: true, shared: false });
      hoisted.mStatus.mockResolvedValue(moving(1));
      vi.useFakeTimers({ shouldAdvanceTime: true });
      renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
      expect(await screen.findByTestId('memory-migration-running')).toBeInTheDocument();

      // A run ends; the read that follows is slow to answer.
      hoisted.mStatus.mockResolvedValueOnce(MOVE_IDLE);
      let answerRead: (value: unknown) => void = () => {};
      hoisted.mStatus.mockReturnValueOnce(
        new Promise(resolve => {
          answerRead = resolve;
        })
      );
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MIGRATION_POLL_MS + 10);
      });
      // Meanwhile the user starts the move again.
      fireEvent.click(await screen.findByTestId('memory-migration-start'));
      expect(await screen.findByTestId('memory-migration-running')).toHaveTextContent('4');

      // The read answers last, with the state from before the start.
      await act(async () => {
        answerRead(MOVE_IDLE);
      });
      expect(screen.getByTestId('memory-migration-running')).toBeInTheDocument();
      expect(screen.queryByTestId('memory-migration-offer')).not.toBeInTheDocument();
    });

    it('keeps the offer when a stale read fails after a failed start', async () => {
      hoisted.mScan.mockResolvedValue({ needed: true, shared: false });
      hoisted.mStatus.mockResolvedValue(moving(1));
      vi.useFakeTimers({ shouldAdvanceTime: true });
      renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
      expect(await screen.findByTestId('memory-migration-running')).toBeInTheDocument();

      // A run ends; the read that follows is slow and will fail.
      hoisted.mStatus.mockResolvedValueOnce(MOVE_IDLE);
      let failRead: (reason: unknown) => void = () => {};
      hoisted.mScan.mockReturnValueOnce(
        new Promise((_, reject) => {
          failRead = reject;
        })
      );
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MIGRATION_POLL_MS + 10);
      });
      // The user starts again, and that start fails.
      hoisted.mStart.mockRejectedValueOnce(new Error('boom'));
      fireEvent.click(await screen.findByTestId('memory-migration-start'));
      expect(await screen.findByTestId('memory-import-error')).toBeInTheDocument();

      // The stale read fails last: the offer must stay, not vanish for a retry.
      await act(async () => {
        failRead(new Error('stale'));
      });
      expect(screen.getByTestId('memory-migration-start')).toBeInTheDocument();
    });

    it('drops a poll that fails after the user started the move', async () => {
      hoisted.mScan.mockResolvedValue({ needed: true, shared: false });
      vi.useFakeTimers({ shouldAdvanceTime: true });
      renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
      expect(await screen.findByTestId('memory-migration-offer')).toBeInTheDocument();

      let failPoll: (reason: unknown) => void = () => {};
      hoisted.mStatus.mockReturnValueOnce(
        new Promise((_, reject) => {
          failPoll = reject;
        })
      );
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MIGRATION_IDLE_POLL_MS + 10);
      });
      fireEvent.click(screen.getByTestId('memory-migration-start'));
      expect(await screen.findByTestId('memory-migration-running')).toBeInTheDocument();

      await act(async () => {
        failPoll(new Error('stale'));
      });
      expect(screen.queryByTestId('memory-import-error')).not.toBeInTheDocument();
    });

    it('scans again after a failed scan', async () => {
      hoisted.mScan
        .mockRejectedValueOnce(new Error('not ready'))
        .mockResolvedValue({ needed: true, shared: false });
      vi.useFakeTimers({ shouldAdvanceTime: true });
      renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
      await waitFor(() => expect(hoisted.mScan).toHaveBeenCalledTimes(1));
      expect(screen.queryByTestId('memory-migration-banner')).not.toBeInTheDocument();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MIGRATION_IDLE_POLL_MS + 10);
      });
      expect(await screen.findByTestId('memory-migration-offer')).toBeInTheDocument();
    });
  });
});
