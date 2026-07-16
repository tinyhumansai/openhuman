import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import * as service from '../../../services/memorySourcesService';
import { renderWithProviders } from '../../../test/test-utils';
import { CodingSessionsCard } from '../CodingSessionsCard';

vi.mock('../../../services/memorySourcesService', async () => {
  const actual = await vi.importActual<typeof import('../../../services/memorySourcesService')>(
    '../../../services/memorySourcesService'
  );
  return { ...actual, getCodingSessionStatus: vi.fn(), ingestCodingSessions: vi.fn() };
});

const mockedStatus = vi.mocked(service.getCodingSessionStatus);
const mockedIngest = vi.mocked(service.ingestCodingSessions);

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

  it('ingests incrementally and reports the distilled observations', async () => {
    mockedIngest.mockResolvedValue({
      mode: 'incremental',
      files_seen: 5,
      sessions_processed: 4,
      sessions_skipped: 1,
      sessions_failed: 0,
      evidence_units: 11,
      observations: 6,
      budget_hit: false,
      pack_path: '/workspace/persona/PERSONA.md',
    });
    const onToast = vi.fn();
    renderWithProviders(<CodingSessionsCard onToast={onToast} />);

    fireEvent.click(await screen.findByTestId('coding-sessions-ingest'));

    await waitFor(() => expect(mockedIngest).toHaveBeenCalledWith(false));
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

  it('warns when more coding sessions remain after the current batch', async () => {
    mockedIngest.mockResolvedValue({
      mode: 'incremental',
      files_seen: 30,
      sessions_processed: 15,
      sessions_skipped: 0,
      sessions_failed: 0,
      evidence_units: 40,
      observations: 20,
      budget_hit: true,
      pack_path: '/workspace/persona/PERSONA.md',
    });
    const onToast = vi.fn();
    renderWithProviders(<CodingSessionsCard onToast={onToast} />);

    fireEvent.click(await screen.findByTestId('coding-sessions-ingest'));

    await waitFor(() =>
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'warning',
          message:
            'The session batch limit was reached. Run ingestion again to continue importing your history.',
        })
      )
    );
  });

  it('reports partial session failures in the warning toast', async () => {
    mockedIngest.mockResolvedValue({
      mode: 'incremental',
      files_seen: 5,
      sessions_processed: 3,
      sessions_skipped: 0,
      sessions_failed: 2,
      evidence_units: 8,
      observations: 4,
      budget_hit: false,
      pack_path: '/workspace/persona/PERSONA.md',
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

  it('reports ingestion failures through the error toast', async () => {
    mockedIngest.mockRejectedValue(new Error('persona pipeline failed'));
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
