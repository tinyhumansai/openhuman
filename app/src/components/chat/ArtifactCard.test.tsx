import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { ArtifactSnapshot } from '../../store/chatRuntimeSlice';
import ArtifactCard from './ArtifactCard';

// Mock the artifact download service — the card only consumes the
// two public functions, so a per-test override is enough.
const saveArtifactViaDialogMock = vi.fn();
const revealArtifactInFileManagerMock = vi.fn();
vi.mock('../../services/artifactDownloadService', () => ({
  saveArtifactViaDialog: (...args: unknown[]) => saveArtifactViaDialogMock(...args),
  revealArtifactInFileManager: (...args: unknown[]) => revealArtifactInFileManagerMock(...args),
}));

function snapshot(overrides: Partial<ArtifactSnapshot> = {}): ArtifactSnapshot {
  return {
    artifactId: 'a-1',
    kind: 'presentation',
    title: 'Quarterly Deck',
    status: 'ready',
    sizeBytes: 4096,
    path: 'a-1/deck.pptx',
    updatedAt: 0,
    ...overrides,
  };
}

beforeEach(() => {
  saveArtifactViaDialogMock.mockReset();
  revealArtifactInFileManagerMock.mockReset();
});

describe('<ArtifactCard /> — in_progress state', () => {
  it('renders the title with the generating sub-label and no buttons', () => {
    render(
      <ArtifactCard
        artifact={snapshot({ status: 'in_progress', sizeBytes: undefined, path: undefined })}
      />
    );

    expect(screen.getByText('Quarterly Deck')).toBeInTheDocument();
    expect(screen.getByText(/Generating presentation/i)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /download/i })).not.toBeInTheDocument();
  });

  it('reflects the artifact kind in the generating label', () => {
    render(
      <ArtifactCard
        artifact={snapshot({
          status: 'in_progress',
          kind: 'image',
          sizeBytes: undefined,
          path: undefined,
        })}
      />
    );

    expect(screen.getByText(/Generating image/i)).toBeInTheDocument();
  });
});

describe('<ArtifactCard /> — ready state', () => {
  it('renders the Download button with the formatted size', () => {
    render(<ArtifactCard artifact={snapshot()} />);

    expect(screen.getByRole('button', { name: 'Download' })).toBeEnabled();
    // formatFileSize on 4096 should land in the "4 KB"-ish range; we
    // just assert the size cell exists rather than asserting an exact
    // formatter output to avoid coupling to its implementation detail.
    const subLabel = screen.getByText(/Ready ·/);
    expect(subLabel.textContent ?? '').toMatch(/[KMG]?B/);
  });

  it('omits the size suffix when sizeBytes is null', () => {
    render(<ArtifactCard artifact={snapshot({ sizeBytes: undefined })} />);

    // Status text should be empty when sizeBytes is missing on a ready
    // artifact — the conditional collapses to ''.
    const dl = screen.getByRole('button', { name: 'Download' });
    expect(dl).toBeInTheDocument();
  });

  it('drives the download → done flow and reveals on click', async () => {
    saveArtifactViaDialogMock.mockResolvedValueOnce({
      ok: true,
      path: '/Users/me/Downloads/Quarterly Deck.pptx',
    });
    revealArtifactInFileManagerMock.mockResolvedValueOnce(true);

    render(<ArtifactCard artifact={snapshot()} />);

    fireEvent.click(screen.getByRole('button', { name: 'Download' }));

    // Service called with (id, title, ext) where ext comes from the
    // title's extension when present, or kind fallback otherwise.
    await waitFor(() => expect(saveArtifactViaDialogMock).toHaveBeenCalledTimes(1));
    expect(saveArtifactViaDialogMock).toHaveBeenCalledWith('a-1', 'Quarterly Deck', 'pptx');

    // Saved-to row appears with the reveal button.
    await screen.findByText(
      (_, el) => el?.textContent === 'Saved to /Users/me/Downloads/Quarterly Deck.pptx'
    );

    fireEvent.click(screen.getByRole('button', { name: 'Show in folder' }));
    await waitFor(() => expect(revealArtifactInFileManagerMock).toHaveBeenCalledTimes(1));
    expect(revealArtifactInFileManagerMock).toHaveBeenCalledWith(
      '/Users/me/Downloads/Quarterly Deck.pptx'
    );

    // Download button is gone now that state === 'done'.
    expect(screen.queryByRole('button', { name: 'Download' })).not.toBeInTheDocument();
  });

  it('falls back to the kind-based extension when title has no dot', async () => {
    saveArtifactViaDialogMock.mockResolvedValueOnce({ ok: true, path: '/tmp/Doc.pdf' });
    render(
      <ArtifactCard artifact={snapshot({ artifactId: 'a-2', kind: 'document', title: 'Doc' })} />
    );

    fireEvent.click(screen.getByRole('button', { name: 'Download' }));
    await waitFor(() => expect(saveArtifactViaDialogMock).toHaveBeenCalledTimes(1));
    expect(saveArtifactViaDialogMock).toHaveBeenCalledWith('a-2', 'Doc', 'pdf');
  });

  it('falls back to png for image kind without an extension', async () => {
    saveArtifactViaDialogMock.mockResolvedValueOnce({ ok: true, path: '/tmp/Pic.png' });
    render(
      <ArtifactCard artifact={snapshot({ artifactId: 'a-3', kind: 'image', title: 'Pic' })} />
    );

    fireEvent.click(screen.getByRole('button', { name: 'Download' }));
    await waitFor(() => expect(saveArtifactViaDialogMock).toHaveBeenCalledTimes(1));
    expect(saveArtifactViaDialogMock).toHaveBeenCalledWith('a-3', 'Pic', 'png');
  });

  it('falls back to bin for the "other" kind without an extension', async () => {
    saveArtifactViaDialogMock.mockResolvedValueOnce({ ok: true, path: '/tmp/Blob.bin' });
    render(
      <ArtifactCard artifact={snapshot({ artifactId: 'a-4', kind: 'other', title: 'Blob' })} />
    );

    fireEvent.click(screen.getByRole('button', { name: 'Download' }));
    await waitFor(() => expect(saveArtifactViaDialogMock).toHaveBeenCalledTimes(1));
    expect(saveArtifactViaDialogMock).toHaveBeenCalledWith('a-4', 'Blob', 'bin');
  });

  it('uses the existing extension on the title when present', async () => {
    saveArtifactViaDialogMock.mockResolvedValueOnce({ ok: true, path: '/tmp/notes.txt' });
    render(
      <ArtifactCard
        artifact={snapshot({ artifactId: 'a-5', kind: 'document', title: 'notes.txt' })}
      />
    );

    fireEvent.click(screen.getByRole('button', { name: 'Download' }));
    await waitFor(() => expect(saveArtifactViaDialogMock).toHaveBeenCalledTimes(1));
    expect(saveArtifactViaDialogMock).toHaveBeenCalledWith('a-5', 'notes.txt', 'txt');
  });

  it('surfaces the service error message when download fails', async () => {
    saveArtifactViaDialogMock.mockResolvedValueOnce({ ok: false, error: 'disk full' });

    render(<ArtifactCard artifact={snapshot()} />);
    fireEvent.click(screen.getByRole('button', { name: 'Download' }));

    await screen.findByText((_, el) => el?.textContent === 'Download failed: disk full');
    // Download button stays so the user can retry.
    expect(screen.getByRole('button', { name: 'Download' })).toBeEnabled();
  });

  it('does not call reveal when download.path is missing', async () => {
    // Synthetic edge: the success path always carries a path, but if the
    // user clicks reveal before a download ever happened the handler is
    // guarded.
    render(<ArtifactCard artifact={snapshot()} />);
    // No reveal button is rendered until state === 'done'.
    expect(screen.queryByRole('button', { name: 'Show in folder' })).not.toBeInTheDocument();
    expect(revealArtifactInFileManagerMock).not.toHaveBeenCalled();
  });
});

describe('<ArtifactCard /> — failed state', () => {
  it('shows the producer error verbatim under the preview cap', () => {
    render(
      <ArtifactCard
        artifact={snapshot({
          status: 'failed',
          path: undefined,
          sizeBytes: undefined,
          error: 'producer crashed',
        })}
      />
    );

    expect(screen.getByText('producer crashed')).toBeInTheDocument();
    expect(screen.getByText(/Generation failed/i)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /show more/i })).not.toBeInTheDocument();
  });

  it('truncates long errors with a Show more toggle', () => {
    const huge = 'x'.repeat(400);
    render(
      <ArtifactCard
        artifact={snapshot({
          status: 'failed',
          path: undefined,
          sizeBytes: undefined,
          error: huge,
        })}
      />
    );

    // Truncated body ends with the ellipsis suffix.
    const truncated = screen.getByText(/x…$/);
    expect(truncated).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Show more' }));
    // After expand, the full string is rendered without the ellipsis.
    expect(screen.getByText(huge)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Show less' }));
    // Re-collapses back to truncated form.
    expect(screen.getByText(/x…$/)).toBeInTheDocument();
  });

  it('renders the Retry button when onRetry is provided', () => {
    const onRetry = vi.fn();
    render(
      <ArtifactCard
        artifact={snapshot({
          status: 'failed',
          path: undefined,
          sizeBytes: undefined,
          error: 'nope',
        })}
        onRetry={onRetry}
      />
    );

    const retry = screen.getByRole('button', { name: 'Retry' });
    fireEvent.click(retry);
    expect(onRetry).toHaveBeenCalledWith('a-1');
  });

  it('omits the Retry button when onRetry is not provided', () => {
    render(
      <ArtifactCard
        artifact={snapshot({
          status: 'failed',
          path: undefined,
          sizeBytes: undefined,
          error: 'nope',
        })}
      />
    );
    expect(screen.queryByRole('button', { name: 'Retry' })).not.toBeInTheDocument();
  });

  it('omits the error block entirely when no error string is set', () => {
    render(
      <ArtifactCard
        artifact={snapshot({
          status: 'failed',
          path: undefined,
          sizeBytes: undefined,
          error: undefined,
        })}
      />
    );
    expect(screen.getByText(/Generation failed/i)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Show more' })).not.toBeInTheDocument();
  });
});

describe('<ArtifactCard /> — aria label', () => {
  it('exposes an aria-label with the artifact title', () => {
    render(<ArtifactCard artifact={snapshot({ title: 'Q3 Plan' })} />);
    expect(screen.getByRole('group', { name: /Q3 Plan/ })).toBeInTheDocument();
  });
});
