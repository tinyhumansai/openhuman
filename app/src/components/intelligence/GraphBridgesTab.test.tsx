import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeGraphBridges } from '../../lib/memory/graphBridges';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import GraphBridgesTab from './GraphBridgesTab';

const mockLoadBridges = vi.fn();
const mockLoadNamespaces = vi.fn();

vi.mock('../../services/api/graphBridgesApi', () => ({
  loadBridges: (...args: unknown[]) => mockLoadBridges(...args),
  loadNamespaces: (...args: unknown[]) => mockLoadNamespaces(...args),
}));

function rel(subject: string, object: string): GraphRelation {
  return {
    namespace: 'n',
    subject,
    predicate: 'p',
    object,
    attrs: {},
    updatedAt: 0,
    evidenceCount: 1,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
  };
}

const result = computeGraphBridges([rel('A', 'B'), rel('B', 'C')]);

describe('<GraphBridgesTab />', () => {
  beforeEach(() => {
    mockLoadBridges.mockReset();
    mockLoadNamespaces.mockReset();
    mockLoadBridges.mockResolvedValue(result);
    mockLoadNamespaces.mockResolvedValue([]);
  });

  it('loads bridges (all namespaces) on mount and renders the result', async () => {
    render(<GraphBridgesTab />);
    expect(mockLoadBridges).toHaveBeenCalledWith(undefined);
    await waitFor(() => expect(screen.getByText('Articulation entities')).toBeInTheDocument());
  });

  it('shows the namespace selector and re-queries on change', async () => {
    mockLoadNamespaces.mockResolvedValueOnce(['work', 'personal']);
    render(<GraphBridgesTab />);
    await waitFor(() => screen.getByRole('combobox'));
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'work' } });
    await waitFor(() => expect(mockLoadBridges).toHaveBeenCalledWith('work'));
  });

  it('surfaces an error when the load fails', async () => {
    mockLoadBridges.mockReset();
    mockLoadBridges.mockRejectedValueOnce(new Error('graph unavailable'));
    render(<GraphBridgesTab />);
    await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/));
  });
});
