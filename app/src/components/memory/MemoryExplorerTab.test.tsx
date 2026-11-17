import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { ExplorePage, Facet, Hit } from '../../services/api/memoryApi';
import { renderWithProviders } from '../../test/test-utils';
import MemoryExplorerTab from './MemoryExplorerTab';
import { facetValueLabel, nextFacet } from './memoryFacetLabels';
import { metaRows } from './MemoryItemDialog';

const hoisted = vi.hoisted(() => ({
  explore: vi.fn(),
  list: vi.fn(),
  get: vi.fn(),
  forget: vi.fn(),
}));

vi.mock('../../services/api/memoryApi', async importOriginal => ({
  ...(await importOriginal<typeof import('../../services/api/memoryApi')>()),
  memoryExplore: (...a: unknown[]) => hoisted.explore(...a),
  memoryItemsList: (...a: unknown[]) => hoisted.list(...a),
  memoryItemsGet: (...a: unknown[]) => hoisted.get(...a),
  memoryForget: (...a: unknown[]) => hoisted.forget(...a),
}));

function page(facet: ExplorePage['facet'], buckets: Array<[string, number]>, extra = {}) {
  return {
    facet,
    buckets: buckets.map(([value, count]) => ({ value, count })),
    total: buckets.reduce((sum, [, count]) => sum + count, 0),
    missing: 0,
    more_buckets: 0,
    truncated: false,
    ...extra,
  };
}

function doc(id: string, text: string): Hit {
  return {
    id,
    kind: 'document',
    text,
    meta: { folder: '/notes', file_path: `/notes/${id}.md`, source: { kind: 'folder', id: 's1' } },
    score: 0,
  };
}

beforeEach(() => {
  hoisted.explore.mockReset();
  hoisted.list.mockReset();
  hoisted.get.mockReset();
  hoisted.forget.mockReset();
  hoisted.explore.mockImplementation(({ facet }: { facet: ExplorePage['facet'] }) =>
    Promise.resolve(
      facet === 'kind'
        ? page('kind', [
            ['document', 3],
            ['learning', 1],
          ])
        : page(facet, [['s1', 3]])
    )
  );
  hoisted.list.mockResolvedValue({ items: [doc('d1', 'Aurora launch plan')] });
});

describe('MemoryExplorerTab', () => {
  it('groups everything by kind and lists the items at the root', async () => {
    renderWithProviders(<MemoryExplorerTab />);
    const bucket = await screen.findByTestId('memory-explorer-bucket-document');
    expect(bucket).toHaveTextContent('Document');
    expect(bucket).toHaveTextContent('3');
    expect(hoisted.explore).toHaveBeenCalledWith({ facet: 'kind', path: [], limit: 50 });
    expect(await screen.findByTestId('memory-explorer-item-d1')).toHaveTextContent(
      'Aurora launch plan'
    );
    expect(hoisted.list).toHaveBeenCalledWith({ path: [], limit: 20, preview: true });
    expect(screen.getByTestId('memory-explorer-root')).toBeDisabled();
  });

  it('drills into a bucket, suggests the next facet, and walks back by breadcrumb', async () => {
    renderWithProviders(<MemoryExplorerTab />);
    fireEvent.click(await screen.findByTestId('memory-explorer-bucket-document'));

    const path = [{ facet: 'kind', value: 'document' }];
    await waitFor(() =>
      expect(hoisted.explore).toHaveBeenLastCalledWith({ facet: 'source', path, limit: 50 })
    );
    expect(hoisted.list).toHaveBeenLastCalledWith({ path, limit: 20, preview: true });
    expect(screen.getByTestId('memory-explorer-step-0')).toHaveTextContent('Document');
    const select = screen.getByTestId('memory-explorer-facet') as HTMLSelectElement;
    expect(select.value).toBe('source');
    expect(within(select).queryByRole('option', { name: 'Type' })).not.toBeInTheDocument();

    fireEvent.change(select, { target: { value: 'folder' } });
    await waitFor(() =>
      expect(hoisted.explore).toHaveBeenLastCalledWith({ facet: 'folder', path, limit: 50 })
    );

    fireEvent.click(screen.getByTestId('memory-explorer-root'));
    await waitFor(() =>
      expect(hoisted.explore).toHaveBeenLastCalledWith({ facet: 'kind', path: [], limit: 50 })
    );
    expect(screen.queryByTestId('memory-explorer-step-0')).not.toBeInTheDocument();
  });

  it('notes missing values, cut buckets and a truncated scan', async () => {
    hoisted.explore.mockResolvedValue(
      page('kind', [['document', 2]], { missing: 4, more_buckets: 7, truncated: true })
    );
    renderWithProviders(<MemoryExplorerTab />);
    expect(await screen.findByTestId('memory-explorer-missing')).toHaveTextContent('4');
    expect(screen.getByTestId('memory-explorer-more-buckets')).toHaveTextContent('7');
    expect(screen.getByTestId('memory-explorer-truncated')).toBeInTheDocument();
  });

  it('says when no item has the facet and when nothing is stored', async () => {
    hoisted.explore.mockResolvedValue(page('kind', []));
    hoisted.list.mockResolvedValue({ items: [] });
    renderWithProviders(<MemoryExplorerTab />);
    expect(await screen.findByTestId('memory-explorer-no-values')).toBeInTheDocument();
    expect(await screen.findByTestId('memory-explorer-empty')).toBeInTheDocument();
  });

  it('pages the items with the cursor', async () => {
    hoisted.list
      .mockResolvedValueOnce({ items: [doc('d1', 'one')], next_cursor: 'c2' })
      .mockResolvedValueOnce({ items: [doc('d2', 'two')] });
    renderWithProviders(<MemoryExplorerTab />);
    fireEvent.click(await screen.findByTestId('memory-explorer-more'));
    expect(await screen.findByTestId('memory-explorer-item-d2')).toBeInTheDocument();
    expect(hoisted.list).toHaveBeenLastCalledWith({
      path: [],
      limit: 20,
      cursor: 'c2',
      preview: true,
    });
  });

  it('shows an explore failure with a retry', async () => {
    hoisted.explore.mockRejectedValueOnce(new Error('engine down'));
    renderWithProviders(<MemoryExplorerTab />);
    const alert = await screen.findByTestId('memory-explorer-error');
    expect(alert).toHaveTextContent('engine down');
    fireEvent.click(within(alert).getByRole('button'));
    expect(await screen.findByTestId('memory-explorer-bucket-document')).toBeInTheDocument();
  });

  it('opens an item whole and forgets it', async () => {
    hoisted.get.mockResolvedValue({
      items: [
        {
          ...doc('d1', 'Aurora launch plan, in full'),
          meta: {
            ...doc('d1', '').meta,
            tags: ['launch', 'q3'],
            tool_call: { name: 'memory', id: 'call-1' },
          },
        },
      ],
    });
    hoisted.forget.mockResolvedValue({ forgotten: 1 });
    renderWithProviders(<MemoryExplorerTab />);
    fireEvent.click(await screen.findByTestId('memory-explorer-open-d1'));

    const dialog = await screen.findByTestId('memory-item-dialog');
    expect(await within(dialog).findByTestId('memory-item-text')).toHaveTextContent(
      'Aurora launch plan, in full'
    );
    expect(hoisted.get).toHaveBeenCalledWith(['d1']);
    expect(dialog).toHaveTextContent('launch, q3');
    expect(dialog).toHaveTextContent('memory (call-1)');

    const callsBefore = hoisted.explore.mock.calls.length;
    fireEvent.click(within(dialog).getByTestId('memory-item-forget'));
    await waitFor(() => expect(hoisted.forget).toHaveBeenCalledWith(['d1']));
    await waitFor(() => expect(screen.queryByTestId('memory-item-dialog')).not.toBeInTheDocument());
    await waitFor(() => expect(hoisted.explore.mock.calls.length).toBeGreaterThan(callsBefore));
  });

  it('says when an opened item is gone and keeps a failed forget open', async () => {
    hoisted.get.mockResolvedValueOnce({ items: [] });
    renderWithProviders(<MemoryExplorerTab />);
    fireEvent.click(await screen.findByTestId('memory-explorer-open-d1'));
    expect(await screen.findByTestId('memory-item-missing')).toBeInTheDocument();
    expect(screen.getByTestId('memory-item-forget')).toBeDisabled();
    fireEvent.click(screen.getByTestId('memory-item-close'));

    hoisted.get.mockResolvedValueOnce({ items: [doc('d1', 'x')] });
    hoisted.forget.mockRejectedValueOnce(new Error('refused'));
    fireEvent.click(await screen.findByTestId('memory-explorer-open-d1'));
    fireEvent.click(await screen.findByTestId('memory-item-forget'));
    expect(await screen.findByTestId('memory-item-error')).toHaveTextContent('refused');
    expect(screen.getByTestId('memory-item-dialog')).toBeInTheDocument();
  });

  it('shows a read failure in the dialog', async () => {
    hoisted.get.mockRejectedValueOnce(new Error('cannot read'));
    renderWithProviders(<MemoryExplorerTab />);
    fireEvent.click(await screen.findByTestId('memory-explorer-open-d1'));
    expect(await screen.findByTestId('memory-item-error')).toHaveTextContent('cannot read');
  });
});

describe('explorer helpers', () => {
  const t = (key: string) => key;

  it('suggests the next unused facet, falling back past the drill order', () => {
    expect(nextFacet([], ['kind'])).toBe('kind');
    expect(nextFacet([{ facet: 'kind', value: 'document' }], ['kind'])).toBe('source');
    const order: Facet[] = ['kind', 'source', 'source_id', 'folder', 'file_path', 'thread', 'tag'];
    const drilled = order.map(facet => ({ facet, value: 'x' }));
    expect(nextFacet(drilled, ['kind', 'workspace'])).toBe('workspace');
  });

  it('translates kinds and sources but keeps other values verbatim', () => {
    expect(facetValueLabel('kind', 'learning', t)).toBe('memoryPage.kind.learning');
    expect(facetValueLabel('source', 'agent', t)).toBe('memoryPage.explorer.source.agent');
    expect(facetValueLabel('source', 'folder', t)).toBe('memoryPage.sourceKind.folder');
    expect(facetValueLabel('folder', '/notes', t)).toBe('/notes');
  });

  it('lists only the metadata an item carries', () => {
    expect(metaRows({}, t)).toEqual([]);
    const rows = metaRows(
      {
        source: { kind: 'conversation', id: null },
        thread_id: 't-1',
        turns: { first: 0, last: 3 },
        tool_call: { name: 'calendar', id: '' },
      },
      t
    );
    expect(rows.map(([, value]) => value)).toEqual(['conversation', 't-1', '0–3', 'calendar']);
  });
});
