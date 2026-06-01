import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeFreshness } from '../../lib/memory/memoryFreshness';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import MemoryFreshnessTab from './MemoryFreshnessTab';

const mockLoadFreshness = vi.fn();
const mockLoadNamespaces = vi.fn();

vi.mock('../../services/api/memoryFreshnessApi', () => ({
  loadFreshness: (...args: unknown[]) => mockLoadFreshness(...args),
  loadNamespaces: (...args: unknown[]) => mockLoadNamespaces(...args),
}));

const NOW = 1_700_000_000;

function rel(subject: string, object: string): GraphRelation {
  return {
    namespace: 'n',
    subject,
    predicate: 'p',
    object,
    attrs: {},
    updatedAt: NOW,
    evidenceCount: 1,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
  };
}

const report = computeFreshness([rel('You', 'Berlin')], NOW);

describe('<MemoryFreshnessTab />', () => {
  beforeEach(() => {
    mockLoadFreshness.mockReset();
    mockLoadNamespaces.mockReset();
    mockLoadFreshness.mockResolvedValue(report);
    mockLoadNamespaces.mockResolvedValue([]);
  });

  it('loads freshness (all namespaces) on mount and renders the result', async () => {
    render(<MemoryFreshnessTab />);
    expect(mockLoadFreshness).toHaveBeenCalledTimes(1);
    // Called with (nowSeconds, undefined-namespace).
    expect(mockLoadFreshness.mock.calls[0][1]).toBeUndefined();
    await waitFor(() => expect(screen.getByText('Re-confirm queue')).toBeInTheDocument());
  });

  it('shows the namespace selector and re-queries on change', async () => {
    mockLoadNamespaces.mockResolvedValueOnce(['work', 'personal']);
    render(<MemoryFreshnessTab />);
    await waitFor(() => screen.getByRole('combobox'));
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'work' } });
    await waitFor(() => expect(mockLoadFreshness).toHaveBeenCalledTimes(2));
    expect(mockLoadFreshness.mock.calls[1][1]).toBe('work');
  });

  it('surfaces an error when the load fails', async () => {
    mockLoadFreshness.mockReset();
    mockLoadFreshness.mockRejectedValueOnce(new Error('graph unavailable'));
    render(<MemoryFreshnessTab />);
    await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/));
  });
});
