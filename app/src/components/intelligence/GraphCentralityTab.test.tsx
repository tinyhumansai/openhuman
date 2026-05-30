import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeGraphCentrality } from '../../lib/memory/graphCentrality';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import GraphCentralityTab from './GraphCentralityTab';

const mockLoadCentrality = vi.fn();
const mockLoadNamespaces = vi.fn();

vi.mock('../../services/api/graphCentralityApi', () => ({
  loadCentrality: (...args: unknown[]) => mockLoadCentrality(...args),
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

const result = computeGraphCentrality([rel('A', 'B'), rel('B', 'A')]);

describe('<GraphCentralityTab />', () => {
  beforeEach(() => {
    mockLoadCentrality.mockReset();
    mockLoadNamespaces.mockReset();
    mockLoadCentrality.mockResolvedValue(result);
    mockLoadNamespaces.mockResolvedValue([]);
  });

  it('loads centrality (all namespaces) on mount and renders the result', async () => {
    render(<GraphCentralityTab />);
    expect(mockLoadCentrality).toHaveBeenCalledWith(undefined);
    await waitFor(() => expect(screen.getByText('Top entities by influence')).toBeInTheDocument());
  });

  it('shows the namespace selector and re-queries on change', async () => {
    mockLoadNamespaces.mockResolvedValueOnce(['work', 'personal']);
    render(<GraphCentralityTab />);
    await waitFor(() => screen.getByRole('combobox'));
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'work' } });
    await waitFor(() => expect(mockLoadCentrality).toHaveBeenCalledWith('work'));
  });

  it('surfaces an error when the load fails', async () => {
    mockLoadCentrality.mockReset();
    mockLoadCentrality.mockRejectedValueOnce(new Error('graph unavailable'));
    render(<GraphCentralityTab />);
    await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/));
  });
});
