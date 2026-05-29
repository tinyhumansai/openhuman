import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computePredicateDiversity } from '../../lib/memory/predicateDiversity';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import PredicateDiversityTab from './PredicateDiversityTab';

const mockLoadDiversity = vi.fn();
const mockLoadNamespaces = vi.fn();

vi.mock('../../services/api/predicateDiversityApi', () => ({
  loadDiversity: (...args: unknown[]) => mockLoadDiversity(...args),
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

const result = computePredicateDiversity([rel('A', 'knows', 'B'), rel('A', 'likes', 'B')]);

describe('<PredicateDiversityTab />', () => {
  beforeEach(() => {
    mockLoadDiversity.mockReset();
    mockLoadNamespaces.mockReset();
    mockLoadDiversity.mockResolvedValue(result);
    mockLoadNamespaces.mockResolvedValue([]);
  });

  it('loads diversity (all namespaces) on mount and renders the result', async () => {
    render(<PredicateDiversityTab />);
    expect(mockLoadDiversity).toHaveBeenCalledWith(undefined);
    await waitFor(() => expect(screen.getByText('Top predicates')).toBeInTheDocument());
  });

  it('shows the namespace selector and re-queries on change', async () => {
    mockLoadNamespaces.mockResolvedValueOnce(['work', 'personal']);
    render(<PredicateDiversityTab />);
    await waitFor(() => screen.getByRole('combobox'));
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'work' } });
    await waitFor(() => expect(mockLoadDiversity).toHaveBeenCalledWith('work'));
  });

  it('surfaces an error when the load fails', async () => {
    mockLoadDiversity.mockReset();
    mockLoadDiversity.mockRejectedValueOnce(new Error('graph unavailable'));
    render(<PredicateDiversityTab />);
    await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/));
  });
});
