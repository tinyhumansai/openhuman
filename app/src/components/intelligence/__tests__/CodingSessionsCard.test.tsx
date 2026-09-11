import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import * as service from '../../../services/memorySourcesService';
import { renderWithProviders } from '../../../test/test-utils';
import { CodingSessionsCard } from '../CodingSessionsCard';

vi.mock('../../../services/memorySourcesService', async () => {
  const actual = await vi.importActual<typeof import('../../../services/memorySourcesService')>(
    '../../../services/memorySourcesService'
  );
  return { ...actual, getCodingSessionStatus: vi.fn(), drainCodingSessions: vi.fn() };
});

const mockedStatus = vi.mocked(service.getCodingSessionStatus);
const mockedDrain = vi.mocked(service.drainCodingSessions);

describe('CodingSessionsCard', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockedStatus.mockResolvedValue([
      {
        kind: 'claude_code',
        available: true,
        session_files: 2,
        evidence_units: 4,
        invalid_files: 0,
      },
      { kind: 'codex', available: true, session_files: 3, evidence_units: 7, invalid_files: 0 },
    ]);
  });

  it('shows discovered local session counts', async () => {
    renderWithProviders(<CodingSessionsCard />);

    expect(await screen.findByTestId('coding-session-source-claude_code')).toHaveTextContent(
      '2 sessions · 4 human turns'
    );
    expect(screen.getByTestId('coding-session-source-codex')).toHaveTextContent(
      '3 sessions · 7 human turns'
    );
    expect(screen.getByTestId('coding-sessions-ingest')).toBeEnabled();
    expect(screen.getByTestId('coding-sessions-ingest')).toHaveAttribute(
      'data-analytics-id',
      'brain-sources-coding-sessions-ingest'
    );
  });

  it('drains the whole backlog and reports the distilled observations', async () => {
    mockedDrain.mockResolvedValue({
      passes: 2,
      sessionsProcessed: 4,
      sessionsFailed: 0,
      observations: 6,
      remaining: 0,
      moreRemaining: false,
      timedOut: false,
    });
    const onToast = vi.fn();
    renderWithProviders(<CodingSessionsCard onToast={onToast} />);

    fireEvent.click(await screen.findByTestId('coding-sessions-ingest'));

    await waitFor(() =>
      expect(mockedDrain).toHaveBeenCalledWith(
        expect.objectContaining({
          onProgress: expect.any(Function),
          shouldStop: expect.any(Function),
        })
      )
    );
    await waitFor(() =>
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'success',
          message: '4 sessions produced 6 persona observations.',
        })
      )
    );
  });

  it('keeps ingestion disabled when no human-authored evidence exists', async () => {
    mockedStatus.mockResolvedValue([
      { kind: 'codex', available: false, session_files: 0, evidence_units: 0, invalid_files: 0 },
    ]);
    renderWithProviders(<CodingSessionsCard />);

    expect(await screen.findByText('No local history found')).toBeInTheDocument();
    expect(screen.getByTestId('coding-sessions-ingest')).toBeDisabled();
  });

  it('shows live progress and pauses the drain when the user stops', async () => {
    let finishDrain!: () => void;
    mockedDrain.mockImplementation(({ onProgress } = {}) => {
      onProgress?.({
        passes: 1,
        sessionsProcessed: 5,
        sessionsFailed: 0,
        observations: 3,
        remaining: 25,
        moreRemaining: true,
        timedOut: false,
      });
      return new Promise(resolve => {
        finishDrain = () =>
          resolve({
            passes: 1,
            sessionsProcessed: 5,
            sessionsFailed: 0,
            observations: 3,
            remaining: 25,
            moreRemaining: true,
            timedOut: false,
          });
      });
    });
    const onToast = vi.fn();
    renderWithProviders(<CodingSessionsCard onToast={onToast} />);

    fireEvent.click(await screen.findByTestId('coding-sessions-ingest'));

    // Live progress renders mid-drain.
    expect(await screen.findByTestId('coding-sessions-progress')).toHaveTextContent(
      '5 sessions imported · 3 observations · about 25 left'
    );

    // Stopping mid-drain reports a paused import with the remaining estimate.
    fireEvent.click(await screen.findByTestId('coding-sessions-stop'));
    finishDrain();

    await waitFor(() =>
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'info',
          title: 'Import paused',
          message: 'Imported 5 sessions. Run import again to continue the remaining 25.',
        })
      )
    );
  });

  it('reports a paused import when the backlog remains without a user stop (cap/stall)', async () => {
    // moreRemaining=true with no Stop click — the drain hit the pass cap or
    // stalled. This must NOT be reported as a complete success.
    mockedDrain.mockResolvedValue({
      passes: 2000,
      sessionsProcessed: 30000,
      sessionsFailed: 0,
      observations: 12000,
      remaining: 300,
      moreRemaining: true,
      timedOut: false,
    });
    const onToast = vi.fn();
    renderWithProviders(<CodingSessionsCard onToast={onToast} />);

    fireEvent.click(await screen.findByTestId('coding-sessions-ingest'));

    await waitFor(() =>
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'info',
          title: 'Import paused',
          message: 'Imported 30000 sessions. Run import again to continue the remaining 300.',
        })
      )
    );
  });

  it('reports partial session failures in the warning toast', async () => {
    mockedDrain.mockResolvedValue({
      passes: 1,
      sessionsProcessed: 3,
      sessionsFailed: 2,
      observations: 4,
      remaining: 0,
      moreRemaining: false,
      timedOut: false,
    });
    const onToast = vi.fn();
    renderWithProviders(<CodingSessionsCard onToast={onToast} />);

    fireEvent.click(await screen.findByTestId('coding-sessions-ingest'));

    await waitFor(() =>
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'warning',
          message: '2 sessions failed while 3 were processed. Run ingestion again to retry them.',
        })
      )
    );
  });

  it('shows status failures as an alert', async () => {
    mockedStatus.mockRejectedValue(new Error('session scan failed'));
    renderWithProviders(<CodingSessionsCard />);

    expect(await screen.findByRole('alert')).toHaveTextContent('session scan failed');
  });

  it('does not report a deadline as a failure banner', async () => {
    // The defect this test exists for (#5802): the import completes seconds
    // after the caller stops waiting, and the user was shown a red "ingestion
    // failed" banner over work that had succeeded — then invited to redo a
    // 35-60s job. A deadline must read as "still running", never as a failure.
    mockedDrain.mockResolvedValue({
      passes: 1,
      sessionsProcessed: 3,
      sessionsFailed: 0,
      observations: 15,
      remaining: 419,
      moreRemaining: true,
      timedOut: true,
    });
    const onToast = vi.fn();
    renderWithProviders(<CodingSessionsCard onToast={onToast} />);

    fireEvent.click(await screen.findByTestId('coding-sessions-ingest'));

    await waitFor(() =>
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'info',
          title: 'Import still running',
          message: expect.stringContaining('Sessions imported so far: 3'),
        })
      )
    );
    // No error toast and no inline alert — both would tell the user the import
    // is over when it is not.
    expect(onToast).not.toHaveBeenCalledWith(expect.objectContaining({ type: 'error' }));
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('leads with the deadline when a stale failure count is also present', async () => {
    // `sessionsFailed` carries the last *completed* pass's count, so on a
    // timeout it describes superseded work. Reporting it would tell the user to
    // retry a run that has not finished.
    mockedDrain.mockResolvedValue({
      passes: 2,
      sessionsProcessed: 4,
      sessionsFailed: 2,
      observations: 9,
      remaining: 100,
      moreRemaining: true,
      timedOut: true,
    });
    const onToast = vi.fn();
    renderWithProviders(<CodingSessionsCard onToast={onToast} />);

    fireEvent.click(await screen.findByTestId('coding-sessions-ingest'));

    await waitFor(() =>
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({ type: 'info', title: 'Import still running' })
      )
    );
    expect(onToast).not.toHaveBeenCalledWith(expect.objectContaining({ type: 'warning' }));
  });

  it('reports ingestion failures through the error toast', async () => {
    mockedDrain.mockRejectedValue(new Error('persona pipeline failed'));
    const onToast = vi.fn();
    renderWithProviders(<CodingSessionsCard onToast={onToast} />);

    fireEvent.click(await screen.findByTestId('coding-sessions-ingest'));

    await waitFor(() =>
      expect(onToast).toHaveBeenCalledWith({
        type: 'error',
        title: 'Coding-session ingestion failed',
        message: 'persona pipeline failed',
      })
    );
  });

  it('warns when a source scan reaches its file cap', async () => {
    mockedStatus.mockResolvedValue([
      {
        kind: 'codex',
        available: true,
        session_files: 1000,
        evidence_units: 1200,
        invalid_files: 0,
        scan_truncated: true,
      },
    ]);
    renderWithProviders(<CodingSessionsCard />);

    expect(await screen.findByText('Scan limited to the first 1,000 session files.')).toBeVisible();
  });

  it('keeps ingestion enabled when a capped scan has not found evidence yet', async () => {
    mockedStatus.mockResolvedValue([
      {
        kind: 'codex',
        available: true,
        session_files: 1000,
        evidence_units: 0,
        invalid_files: 1000,
        scan_truncated: true,
      },
    ]);
    renderWithProviders(<CodingSessionsCard />);

    expect(await screen.findByTestId('coding-sessions-ingest')).toBeEnabled();
  });
});
