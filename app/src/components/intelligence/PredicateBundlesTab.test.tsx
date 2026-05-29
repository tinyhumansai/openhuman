import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computePredicateBundles } from '../../lib/memory/predicateBundles';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import PredicateBundlesTab from './PredicateBundlesTab';

const mockLoadBundles = vi.fn();
const mockLoadNamespaces = vi.fn();

vi.mock('../../services/api/predicateBundlesApi', () => ({
  loadBundles: (...args: unknown[]) => mockLoadBundles(...args),
  loadNamespaces: (...args: unknown[]) => mockLoadNamespaces(...args),
}));

function rel(subject: string, predicate: string, object: string): GraphRelation {
  return {
    namespace: 'n',
    subject,
    predicate,
    object,
    attrs: {},
    updatedAt: 0,
    evidenceCount: 1,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
  };
}

const result = computePredicateBundles([rel('A', 'knows', 'B'), rel('A', 'trusts', 'B')]);

describe('<PredicateBundlesTab />', () => {
  beforeEach(() => {
    mockLoadBundles.mockReset();
    mockLoadNamespaces.mockReset();
    mockLoadBundles.mockResolvedValue(result);
    mockLoadNamespaces.mockResolvedValue([]);
  });

  it('loads bundles (all namespaces) on mount and renders the result', async () => {
    render(<PredicateBundlesTab />);
    expect(mockLoadBundles).toHaveBeenCalledWith(undefined);
    await waitFor(() => expect(screen.getByText('Top bundles')).toBeInTheDocument());
  });

  it('shows the namespace selector and re-queries on change', async () => {
    mockLoadNamespaces.mockResolvedValueOnce(['work', 'personal']);
    render(<PredicateBundlesTab />);
    await waitFor(() => screen.getByRole('combobox'));
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'work' } });
    await waitFor(() => expect(mockLoadBundles).toHaveBeenCalledWith('work'));
  });

  it('surfaces an error when the load fails', async () => {
    mockLoadBundles.mockReset();
    mockLoadBundles.mockRejectedValueOnce(new Error('graph unavailable'));
    render(<PredicateBundlesTab />);
    await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/));
  });
});
