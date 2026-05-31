import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeGraphReach } from '../../lib/memory/graphReach';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import GraphReachTab from './GraphReachTab';

const mockLoadReach = vi.fn();
const mockLoadNamespaces = vi.fn();

vi.mock('../../services/api/graphReachApi', () => ({
  loadReach: (...args: unknown[]) => mockLoadReach(...args),
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

const result = computeGraphReach([rel('A', 'B'), rel('B', 'C'), rel('C', 'D')]);

describe('<GraphReachTab />', () => {
  beforeEach(() => {
    mockLoadReach.mockReset();
    mockLoadNamespaces.mockReset();
    mockLoadReach.mockResolvedValue(result);
    mockLoadNamespaces.mockResolvedValue([]);
  });

  it('loads reach (all namespaces) on mount and renders the result', async () => {
    render(<GraphReachTab />);
    expect(mockLoadReach).toHaveBeenCalledWith(undefined);
    await waitFor(() => expect(screen.getByText('Most central entities')).toBeInTheDocument());
  });

  it('shows the namespace selector and re-queries on change', async () => {
    mockLoadNamespaces.mockResolvedValueOnce(['work', 'personal']);
    render(<GraphReachTab />);
    await waitFor(() => screen.getByRole('combobox'));
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'work' } });
    await waitFor(() => expect(mockLoadReach).toHaveBeenCalledWith('work'));
  });

  it('surfaces an error when the load fails', async () => {
    mockLoadReach.mockReset();
    mockLoadReach.mockRejectedValueOnce(new Error('graph unavailable'));
    render(<GraphReachTab />);
    await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/));
  });
});
