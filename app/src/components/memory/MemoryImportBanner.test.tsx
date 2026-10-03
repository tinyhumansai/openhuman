import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import MemoryImportBanner, { IMPORT_POLL_MS } from './MemoryImportBanner';

const hoisted = vi.hoisted(() => ({ scan: vi.fn(), start: vi.fn(), status: vi.fn() }));

vi.mock('../../services/api/memoryApi', async importOriginal => ({
  ...(await importOriginal<typeof import('../../services/api/memoryApi')>()),
  memoryImportScan: (...a: unknown[]) => hoisted.scan(...a),
  memoryImportStart: (...a: unknown[]) => hoisted.start(...a),
  memoryImportStatus: (...a: unknown[]) => hoisted.status(...a),
}));

const IDLE = { state: { phase: 'idle', imported: 0, total: 0 } };
const FOUND = { found: true, counts: { documents: 3, conversations: 5, learnings: 2 } };

beforeEach(() => {
  hoisted.scan.mockReset().mockResolvedValue(FOUND);
  hoisted.status.mockReset().mockResolvedValue(IDLE);
  hoisted.start.mockReset();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('MemoryImportBanner', () => {
  it('renders nothing when there is nothing to import', async () => {
    hoisted.scan.mockResolvedValue({ found: false });
    renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
    await waitFor(() => expect(hoisted.scan).toHaveBeenCalled());
    expect(screen.queryByTestId('memory-import-banner')).not.toBeInTheDocument();
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

  it('shows a failed import', async () => {
    hoisted.status.mockResolvedValue({
      state: { phase: 'error', imported: 2, total: 9, error: 'engine rejected batch' },
    });
    renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
    expect(await screen.findByTestId('memory-import-error')).toHaveTextContent(
      'engine rejected batch'
    );
  });

  it('shows a start failure', async () => {
    hoisted.start.mockRejectedValue(new Error('UNAUTHORIZED: sign in again'));
    renderWithProviders(<MemoryImportBanner engineLabel="TinyHumans" />);
    fireEvent.click(await screen.findByTestId('memory-import-open'));
    fireEvent.click(screen.getByTestId('memory-import-confirm'));
    expect(await screen.findByTestId('memory-import-error')).toHaveTextContent('sign in again');
  });
});
