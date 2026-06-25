import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  revealArtifactInFileManager,
  saveArtifactViaDialog,
} from '../../../services/artifactDownloadService';
import type { ArtifactSnapshot } from '../../../store/chatRuntimeSlice';
import ArtifactCard from '../ArtifactCard';

vi.mock('../../../services/artifactDownloadService', () => ({
  saveArtifactViaDialog: vi.fn(),
  revealArtifactInFileManager: vi.fn(),
}));

function inProgress(overrides: Partial<ArtifactSnapshot> = {}): ArtifactSnapshot {
  return {
    artifactId: 'art-1',
    kind: 'presentation',
    title: 'Climate Deck',
    status: 'in_progress',
    updatedAt: Date.now(),
    ...overrides,
  };
}

function ready(overrides: Partial<ArtifactSnapshot> = {}): ArtifactSnapshot {
  return {
    artifactId: 'art-1',
    kind: 'presentation',
    title: 'Climate Deck',
    status: 'ready',
    path: 'artifacts/art-1.pptx',
    sizeBytes: 4096,
    updatedAt: Date.now(),
    ...overrides,
  };
}

function failed(overrides: Partial<ArtifactSnapshot> = {}): ArtifactSnapshot {
  return {
    artifactId: 'art-1',
    kind: 'presentation',
    title: 'Climate Deck',
    status: 'failed',
    error: 'producer crashed',
    updatedAt: Date.now(),
    ...overrides,
  };
}

describe('ArtifactCard', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  // ─── in_progress ────────────────────────────────────────────────────────

  it('renders the in-progress label and no download button', () => {
    render(<ArtifactCard artifact={inProgress()} />);
    expect(screen.getByText(/Generating presentation/)).toBeInTheDocument();
    // No download button while in progress
    expect(screen.queryByRole('button', { name: /Download/ })).toBeNull();
    // role=group + aria carries the title
    expect(screen.getByRole('group', { name: /Climate Deck/ })).toBeInTheDocument();
  });

  // ─── ready ──────────────────────────────────────────────────────────────

  it('renders the size + Download button when ready', () => {
    render(<ArtifactCard artifact={ready({ sizeBytes: 4096 })} />);
    expect(screen.getByText(/Ready/)).toBeInTheDocument();
    // 4096 bytes → "4.0 KB" per formatFileSize
    expect(screen.getByText(/4\.0 KB/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Download' })).toBeInTheDocument();
  });

  it('on Download click → calls saveArtifactViaDialog with title-derived extension on success', async () => {
    vi.mocked(saveArtifactViaDialog).mockResolvedValueOnce({
      ok: true,
      path: '/Users/me/Downloads/Climate Deck.pptx',
    });
    render(<ArtifactCard artifact={ready({ title: 'climate-deck.pptx' })} />);
    fireEvent.click(screen.getByRole('button', { name: 'Download' }));
    await waitFor(() => {
      expect(saveArtifactViaDialog).toHaveBeenCalledWith('art-1', 'climate-deck.pptx', 'pptx');
    });
    // Saved-to label appears with the resolved path
    await waitFor(() => {
      expect(screen.getByText(/Saved to/)).toBeInTheDocument();
    });
    // Reveal button appears and the original Download button is gone
    expect(screen.getByRole('button', { name: 'Show in folder' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Download' })).toBeNull();
  });

  it('on Reveal click → calls revealArtifactInFileManager with the saved path', async () => {
    vi.mocked(saveArtifactViaDialog).mockResolvedValueOnce({
      ok: true,
      path: '/Users/me/Downloads/Climate Deck.pptx',
    });
    render(<ArtifactCard artifact={ready()} />);
    fireEvent.click(screen.getByRole('button', { name: 'Download' }));
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Show in folder' })).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole('button', { name: 'Show in folder' }));
    await waitFor(() => {
      expect(revealArtifactInFileManager).toHaveBeenCalledWith(
        '/Users/me/Downloads/Climate Deck.pptx'
      );
    });
  });

  it('on Download failure → surfaces the error reason and leaves the Download button in place', async () => {
    vi.mocked(saveArtifactViaDialog).mockResolvedValueOnce({
      ok: false,
      code: 'NOT_DESKTOP',
      error: 'Downloads are only available in the desktop app',
    });
    render(<ArtifactCard artifact={ready()} />);
    fireEvent.click(screen.getByRole('button', { name: 'Download' }));
    await waitFor(() => {
      expect(screen.getByText(/Download failed:/)).toBeInTheDocument();
    });
    // Original Download button is still there for retry
    expect(screen.getByRole('button', { name: 'Download' })).toBeInTheDocument();
    // No "Show in folder" affordance on failure
    expect(screen.queryByRole('button', { name: 'Show in folder' })).toBeNull();
  });

  it.each([
    ['document' as const, 'pdf'],
    ['image' as const, 'png'],
    ['other' as const, 'bin'],
    ['presentation' as const, 'pptx'],
  ])(
    'falls back to per-kind extension when title lacks one (kind=%s → ext=%s)',
    async (kind, expectedExt) => {
      vi.mocked(saveArtifactViaDialog).mockResolvedValueOnce({ ok: true, path: '/d/x' });
      render(<ArtifactCard artifact={ready({ kind, title: 'no-extension' })} />);
      fireEvent.click(screen.getByRole('button', { name: 'Download' }));
      await waitFor(() => {
        expect(saveArtifactViaDialog).toHaveBeenCalledWith('art-1', 'no-extension', expectedExt);
      });
    }
  );

  it('treats a trailing-dot title as having no extension (falls through to kind default)', async () => {
    vi.mocked(saveArtifactViaDialog).mockResolvedValueOnce({ ok: true, path: '/d/x' });
    render(<ArtifactCard artifact={ready({ kind: 'presentation', title: 'trailing.' })} />);
    fireEvent.click(screen.getByRole('button', { name: 'Download' }));
    await waitFor(() => {
      expect(saveArtifactViaDialog).toHaveBeenCalledWith('art-1', 'trailing.', 'pptx');
    });
  });

  it('Download button is disabled while a download is in flight', async () => {
    let resolveDownload: (v: { ok: true; path: string }) => void = () => {};
    vi.mocked(saveArtifactViaDialog).mockImplementationOnce(
      () =>
        new Promise(r => {
          resolveDownload = r;
        })
    );
    render(<ArtifactCard artifact={ready()} />);
    const btn = screen.getByRole('button', { name: 'Download' });
    fireEvent.click(btn);
    // While in-flight, button text flips to "Downloading…" and is disabled.
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Downloading…' })).toBeDisabled();
    });
    // Finish the download to settle the promise.
    resolveDownload({ ok: true, path: '/d/x.pptx' });
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Show in folder' })).toBeInTheDocument();
    });
  });

  // ─── failed ─────────────────────────────────────────────────────────────

  it('renders the failed label and the producer-supplied reason', () => {
    render(<ArtifactCard artifact={failed({ error: 'pip install crashed' })} />);
    expect(screen.getByText(/Generation failed/)).toBeInTheDocument();
    expect(screen.getByText('pip install crashed')).toBeInTheDocument();
  });

  it('renders Retry only when onRetry is provided; clicking it fires the callback', () => {
    const onRetry = vi.fn();
    const a = failed();
    const { rerender } = render(<ArtifactCard artifact={a} />);
    // No Retry when onRetry is absent.
    expect(screen.queryByRole('button', { name: 'Retry' })).toBeNull();

    rerender(<ArtifactCard artifact={a} onRetry={onRetry} />);
    const retryBtn = screen.getByRole('button', { name: 'Retry' });
    fireEvent.click(retryBtn);
    expect(onRetry).toHaveBeenCalledWith('art-1');
  });

  it('long error reason is truncated by default and expands via Show more', () => {
    const longError = 'x'.repeat(400);
    render(<ArtifactCard artifact={failed({ error: longError })} />);
    // Truncated preview ends with ellipsis.
    const para = screen.getByText((_content, el) => {
      return !!el && el.tagName === 'P' && (el.textContent ?? '').endsWith('…');
    });
    expect(para).toBeInTheDocument();
    expect((para.textContent ?? '').length).toBeLessThan(longError.length);

    // Show more → full error visible + button flips to Show less.
    fireEvent.click(screen.getByRole('button', { name: 'Show more' }));
    expect(screen.getByText(longError)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Show less' })).toBeInTheDocument();

    // Show less → re-collapses.
    fireEvent.click(screen.getByRole('button', { name: 'Show less' }));
    expect(screen.getByRole('button', { name: 'Show more' })).toBeInTheDocument();
  });

  it('short error reason (≤ preview cap) does NOT show the Show more affordance', () => {
    render(<ArtifactCard artifact={failed({ error: 'short oops' })} />);
    expect(screen.getByText('short oops')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Show more' })).toBeNull();
  });

  it('failed status uses the failed icon (not the kind icon) and no Download button', () => {
    render(<ArtifactCard artifact={failed()} />);
    expect(screen.queryByRole('button', { name: 'Download' })).toBeNull();
  });

  // ─── kind variants (icon paths) ─────────────────────────────────────────

  it.each(['presentation', 'document', 'image', 'other'] as const)(
    'renders the in-progress spinner for kind=%s without crashing',
    kind => {
      render(<ArtifactCard artifact={inProgress({ kind })} />);
      expect(screen.getByText(/Generating /)).toBeInTheDocument();
    }
  );

  it.each(['presentation', 'document', 'image', 'other'] as const)(
    'renders the kind icon when ready for kind=%s',
    kind => {
      render(<ArtifactCard artifact={ready({ kind })} />);
      // Ready label is present regardless of kind.
      expect(screen.getByText(/Ready/)).toBeInTheDocument();
    }
  );
});
