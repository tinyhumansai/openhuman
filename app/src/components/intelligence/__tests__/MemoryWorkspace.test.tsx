import { fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import { MemoryWorkspace } from '../MemoryWorkspace';

describe('MemoryWorkspace — three-pane browser', () => {
  it('renders the three pane scaffold and the navigator search box', async () => {
    renderWithProviders(<MemoryWorkspace />);
    expect(screen.getByTestId('memory-workspace')).toBeInTheDocument();
    expect(screen.getByTestId('memory-navigator')).toBeInTheDocument();
    expect(screen.getByTestId('memory-result-list')).toBeInTheDocument();
    expect(screen.getByLabelText('Search memory')).toBeInTheDocument();
  });

  it('renders navigator section headings (recent, sources, people, topics)', async () => {
    renderWithProviders(<MemoryWorkspace />);
    expect(screen.getByText('recent')).toBeInTheDocument();
    expect(screen.getByText('sources')).toBeInTheDocument();
    expect(screen.getByText('people')).toBeInTheDocument();
    expect(screen.getByText('topics')).toBeInTheDocument();
  });

  it('auto-selects the most recent admitted chunk on mount and renders detail', async () => {
    renderWithProviders(<MemoryWorkspace />);
    await waitFor(() => {
      expect(screen.getByTestId('memory-chunk-detail')).toBeInTheDocument();
      expect(screen.getByTestId('memory-chunk-letterhead')).toBeInTheDocument();
    });
  });

  it('renders the result list with TODAY group present at the top', async () => {
    renderWithProviders(<MemoryWorkspace />);
    await waitFor(() => {
      expect(screen.getByText('TODAY')).toBeInTheDocument();
    });
  });

  it('renders source rows for known mock sources', async () => {
    renderWithProviders(<MemoryWorkspace />);
    await waitFor(() => {
      // "Steven Enamakel" can appear as both a source row and a person entity;
      // assert at least one match (use getAllByText which throws on zero).
      expect(screen.getAllByText('Steven Enamakel').length).toBeGreaterThan(0);
    });
    expect(screen.getByText('GitHub notifications')).toBeInTheDocument();
  });

  it('filters the result list when a navigator source is clicked', async () => {
    renderWithProviders(<MemoryWorkspace />);
    await waitFor(() => screen.getByText('GitHub notifications'));

    // Before click — multiple sources surface in the result list
    const beforeRows = screen.getAllByRole('button').filter(b => b.dataset.chunkId);
    expect(beforeRows.length).toBeGreaterThan(1);

    fireEvent.click(screen.getByText('GitHub notifications'));

    await waitFor(() => {
      const rows = screen
        .getAllByRole('button')
        .filter(b => b.dataset.chunkId)
        .map(b => b.textContent ?? '');
      // Every visible row should mention GitHub notifications via meta line
      expect(rows.length).toBeGreaterThan(0);
      expect(rows.every(r => /github/i.test(r))).toBe(true);
    });
  });

  it('typing in the search box narrows the result list', async () => {
    renderWithProviders(<MemoryWorkspace />);
    await waitFor(() => screen.getByText('TODAY'));

    const search = screen.getByLabelText('Search memory') as HTMLInputElement;
    fireEvent.change(search, { target: { value: 'PR #1175' } });

    await waitFor(() => {
      const visible = screen.getAllByRole('button').filter(b => b.dataset.chunkId);
      expect(visible.length).toBeGreaterThan(0);
      expect(visible.length).toBeLessThan(5);
    });
  });

  it('clicking a result row populates the detail pane with that chunk', async () => {
    renderWithProviders(<MemoryWorkspace />);
    await waitFor(() => screen.getByText('TODAY'));

    const rows = screen.getAllByRole('button').filter(b => b.dataset.chunkId);
    expect(rows.length).toBeGreaterThan(1);

    // Pick a different row than the auto-selected one
    const target = rows[rows.length - 1]!;
    const targetId = target.dataset.chunkId!;
    fireEvent.click(target);

    await waitFor(() => {
      const active = document.querySelector('[data-chunk-id].is-active') as HTMLElement | null;
      expect(active?.dataset.chunkId).toBe(targetId);
    });
  });

  it('renders score bars in the detail pane', async () => {
    renderWithProviders(<MemoryWorkspace />);
    await waitFor(() => {
      expect(screen.getByTestId('memory-chunk-scorebars')).toBeInTheDocument();
    });
    // There should be 3 SVG bars — source / entities / recency
    const svgs = screen.getByTestId('memory-chunk-scorebars').querySelectorAll('svg');
    expect(svgs.length).toBe(3);
  });

  it('renders mentioned entities and clicking one activates the lens', async () => {
    renderWithProviders(<MemoryWorkspace />);
    await waitFor(() => screen.getByTestId('memory-chunk-mentioned'));

    const mentionedRows = screen
      .getByTestId('memory-chunk-mentioned')
      .querySelectorAll('button.mw-mentioned-row');
    expect(mentionedRows.length).toBeGreaterThan(0);

    fireEvent.click(mentionedRows[0]!);

    // After click, the result list is narrowed to chunks tagged with that entity
    await waitFor(() => {
      const rows = screen.getAllByRole('button').filter(b => b.dataset.chunkId);
      expect(rows.length).toBeGreaterThan(0);
    });
  });
});

describe('MemoryWorkspace — empty state', () => {
  it('renders the empty placeholder when there are zero chunks', async () => {
    const apiMod = await import('../../../lib/memory/memoryTreeApi');
    const listSpy = vi
      .spyOn(apiMod.memoryTreeApi, 'listChunks')
      .mockResolvedValue({ chunks: [], total: 0 });
    const sourcesSpy = vi.spyOn(apiMod.memoryTreeApi, 'listSources').mockResolvedValue([]);
    const entSpy = vi.spyOn(apiMod.memoryTreeApi, 'topEntities').mockResolvedValue([]);

    renderWithProviders(<MemoryWorkspace />);

    await waitFor(() => {
      expect(screen.getByTestId('memory-empty-placeholder')).toBeInTheDocument();
    });
    expect(screen.getByText('Nothing yet.')).toBeInTheDocument();

    listSpy.mockRestore();
    sourcesSpy.mockRestore();
    entSpy.mockRestore();
  });
});
