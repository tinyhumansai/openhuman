import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeEntityDuplicates } from '../../lib/memory/entityDuplicates';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import EntityDuplicatesTab from './EntityDuplicatesTab';

const mockLoad = vi.fn();
const mockLoadNamespaces = vi.fn();

vi.mock('../../services/api/entityDuplicatesApi', () => ({
  loadEntityDuplicates: (...args: unknown[]) => mockLoad(...args),
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

const report = computeEntityDuplicates([rel('Alice', 'Bob'), rel('alice', 'Carol')]);

describe('<EntityDuplicatesTab />', () => {
  beforeEach(() => {
    mockLoad.mockReset();
    mockLoadNamespaces.mockReset();
    mockLoad.mockResolvedValue(report);
    mockLoadNamespaces.mockResolvedValue([]);
  });

  it('loads on mount and renders the clusters', async () => {
    render(<EntityDuplicatesTab />);
    expect(mockLoad).toHaveBeenCalledWith(undefined);
    await waitFor(() => expect(screen.getByText('Likely duplicate entities')).toBeInTheDocument());
  });

  it('shows the namespace selector and re-queries on change', async () => {
    mockLoadNamespaces.mockResolvedValueOnce(['work', 'personal']);
    render(<EntityDuplicatesTab />);
    await waitFor(() => screen.getByRole('combobox'));
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'work' } });
    await waitFor(() => expect(mockLoad).toHaveBeenCalledWith('work'));
  });

  it('surfaces an error when the load fails', async () => {
    mockLoad.mockReset();
    mockLoad.mockRejectedValueOnce(new Error('graph unavailable'));
    render(<EntityDuplicatesTab />);
    await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/));
  });
});
