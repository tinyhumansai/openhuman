import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeGraphCohesion } from '../../lib/memory/graphCohesion';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import GraphCohesionTab from './GraphCohesionTab';

const mockLoadCohesion = vi.fn();
const mockLoadNamespaces = vi.fn();

vi.mock('../../services/api/graphCohesionApi', () => ({
  loadCohesion: (...args: unknown[]) => mockLoadCohesion(...args),
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

const result = computeGraphCohesion([rel('A', 'B'), rel('B', 'C'), rel('C', 'A')]);

describe('<GraphCohesionTab />', () => {
  beforeEach(() => {
    mockLoadCohesion.mockReset();
    mockLoadNamespaces.mockReset();
    mockLoadCohesion.mockResolvedValue(result);
    mockLoadNamespaces.mockResolvedValue([]);
  });

  it('loads cohesion (all namespaces) on mount and renders the result', async () => {
    render(<GraphCohesionTab />);
    expect(mockLoadCohesion).toHaveBeenCalledWith(undefined);
    await waitFor(() =>
      expect(screen.getByText('Brokers — loosest neighbourhoods')).toBeInTheDocument()
    );
  });

  it('shows the namespace selector and re-queries on change', async () => {
    mockLoadNamespaces.mockResolvedValueOnce(['work', 'personal']);
    render(<GraphCohesionTab />);
    await waitFor(() => screen.getByRole('combobox'));
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'work' } });
    await waitFor(() => expect(mockLoadCohesion).toHaveBeenCalledWith('work'));
  });

  it('surfaces an error when the load fails', async () => {
    mockLoadCohesion.mockReset();
    mockLoadCohesion.mockRejectedValueOnce(new Error('graph unavailable'));
    render(<GraphCohesionTab />);
    await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/));
  });
});
