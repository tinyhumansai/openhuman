import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import ModelCatalog from '../ModelCatalog';

describe('ModelCatalog', () => {
  it('renders one row per recommended model', () => {
    render(
      <ModelCatalog
        installedModelIds={[]}
        activeModelIds={[]}
        onDownload={vi.fn()}
        onUse={vi.fn()}
      />
    );
    // Each id from RECOMMENDED_MODEL_CATALOG appears as a row title.
    expect(screen.getByText('qwen2.5:0.5b')).toBeInTheDocument();
    expect(screen.getByText('gemma3:1b-it-qat')).toBeInTheDocument();
    expect(screen.getByText('gemma3:4b')).toBeInTheDocument();
    expect(screen.getByText('gemma3:12b-it-qat')).toBeInTheDocument();
    expect(screen.getByText('bge-m3')).toBeInTheDocument();
  });

  it('shows "Download" for models that are not installed', () => {
    render(
      <ModelCatalog
        installedModelIds={[]}
        activeModelIds={[]}
        onDownload={vi.fn()}
        onUse={vi.fn()}
      />
    );
    // Five models, all available → five Download buttons.
    expect(screen.getAllByRole('button', { name: /download/i })).toHaveLength(5);
    expect(screen.getAllByText('not downloaded').length).toBeGreaterThan(0);
  });

  it('shows "Use" for installed-but-not-active models, "in use" for active', () => {
    render(
      <ModelCatalog
        installedModelIds={['gemma3:1b-it-qat', 'bge-m3:latest']}
        activeModelIds={['bge-m3']}
        onDownload={vi.fn()}
        onUse={vi.fn()}
      />
    );
    // bge-m3 is installed AND active → "in use" pill, no Use button for it.
    expect(screen.getAllByText('in use').length).toBeGreaterThan(0);
    // gemma3 is installed but not active → Use button visible.
    expect(screen.getByRole('button', { name: 'Use' })).toBeInTheDocument();
  });

  it('matches `bge-m3` against `bge-m3:latest` via the :latest normalization', () => {
    // Ollama tags everything as `:latest` by default; the catalog uses bare
    // names. The component must treat them as the same id.
    render(
      <ModelCatalog
        installedModelIds={['bge-m3:latest']}
        activeModelIds={[]}
        onDownload={vi.fn()}
        onUse={vi.fn()}
      />
    );
    // bge-m3 row is now in the "installed" state — at least one Use button
    // appears (for bge-m3 specifically).
    expect(screen.getByRole('button', { name: 'Use' })).toBeInTheDocument();
  });

  it('fires onUse with the matching model when the Use button is clicked', () => {
    const onUse = vi.fn();
    render(
      <ModelCatalog
        installedModelIds={['gemma3:4b']}
        activeModelIds={[]}
        onDownload={vi.fn()}
        onUse={onUse}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: 'Use' }));
    expect(onUse).toHaveBeenCalledTimes(1);
    expect(onUse.mock.calls[0][0]).toMatchObject({ id: 'gemma3:4b' });
  });

  it('renders Delete buttons only when onDelete is provided', () => {
    const { rerender } = render(
      <ModelCatalog
        installedModelIds={['gemma3:4b']}
        activeModelIds={[]}
        onDownload={vi.fn()}
        onUse={vi.fn()}
      />
    );
    expect(screen.queryByLabelText('Delete model')).toBeNull();

    rerender(
      <ModelCatalog
        installedModelIds={['gemma3:4b']}
        activeModelIds={[]}
        onDownload={vi.fn()}
        onUse={vi.fn()}
        onDelete={vi.fn()}
      />
    );
    expect(screen.getByLabelText('Delete model')).toBeInTheDocument();
  });

  it('shows a progress bar while a download is in flight, then clears it', async () => {
    let resolveDownload!: () => void;
    const onDownload = vi.fn(
      () =>
        new Promise<void>(resolve => {
          resolveDownload = resolve;
        })
    );
    render(
      <ModelCatalog
        installedModelIds={[]}
        activeModelIds={[]}
        onDownload={onDownload}
        onUse={vi.fn()}
      />
    );

    fireEvent.click(screen.getAllByRole('button', { name: /download/i })[0]);

    // Mid-flight: a progressbar is rendered for that row.
    await waitFor(() => {
      expect(screen.queryAllByRole('progressbar').length).toBeGreaterThan(0);
    });

    resolveDownload();
    // After settle (~600 ms on success), the bar disappears and the row
    // returns to its post-install state. We just confirm the state
    // eventually clears — not the exact timing.
    await waitFor(
      () => {
        expect(screen.queryByRole('progressbar')).toBeNull();
      },
      { timeout: 2000 }
    );
  });
});
