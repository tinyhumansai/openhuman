import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeRelationshipTypes } from '../../lib/memory/relationshipTypes';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import RelationshipTypesTab from './RelationshipTypesTab';

const mockLoad = vi.fn();
const mockLoadNamespaces = vi.fn();

vi.mock('../../services/api/relationshipTypesApi', () => ({
  loadRelationshipTypes: (...args: unknown[]) => mockLoad(...args),
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

const report = computeRelationshipTypes([rel('A', 'knows', 'B')]);

describe('<RelationshipTypesTab />', () => {
  beforeEach(() => {
    mockLoad.mockReset();
    mockLoadNamespaces.mockReset();
    mockLoad.mockResolvedValue(report);
    mockLoadNamespaces.mockResolvedValue([]);
  });

  it('loads on mount and renders the ranked list', async () => {
    render(<RelationshipTypesTab />);
    expect(mockLoad).toHaveBeenCalledWith(undefined);
    await waitFor(() => expect(screen.getByText('Most-used relationships')).toBeInTheDocument());
  });

  it('shows the namespace selector and re-queries on change', async () => {
    mockLoadNamespaces.mockResolvedValueOnce(['work', 'personal']);
    render(<RelationshipTypesTab />);
    await waitFor(() => screen.getByRole('combobox'));
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'work' } });
    await waitFor(() => expect(mockLoad).toHaveBeenCalledWith('work'));
  });

  it('surfaces an error when the load fails', async () => {
    mockLoad.mockReset();
    mockLoad.mockRejectedValueOnce(new Error('graph unavailable'));
    render(<RelationshipTypesTab />);
    await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/));
  });
});
