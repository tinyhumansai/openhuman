import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeKnowledgeGaps } from '../../lib/memory/knowledgeGaps';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import KnowledgeGapsTab from './KnowledgeGapsTab';

const mockLoad = vi.fn();
const mockLoadNamespaces = vi.fn();

vi.mock('../../services/api/knowledgeGapsApi', () => ({
  loadKnowledgeGaps: (...args: unknown[]) => mockLoad(...args),
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

const report = computeKnowledgeGaps([rel('A', 'B'), rel('B', 'C')]);

describe('<KnowledgeGapsTab />', () => {
  beforeEach(() => {
    mockLoad.mockReset();
    mockLoadNamespaces.mockReset();
    mockLoad.mockResolvedValue(report);
    mockLoadNamespaces.mockResolvedValue([]);
  });

  it('loads on mount and renders the gap list', async () => {
    render(<KnowledgeGapsTab />);
    expect(mockLoad).toHaveBeenCalledWith(undefined);
    await waitFor(() => expect(screen.getByText('Sparse entities')).toBeInTheDocument());
  });

  it('shows the namespace selector and re-queries on change', async () => {
    mockLoadNamespaces.mockResolvedValueOnce(['work', 'personal']);
    render(<KnowledgeGapsTab />);
    await waitFor(() => screen.getByRole('combobox'));
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'work' } });
    await waitFor(() => expect(mockLoad).toHaveBeenCalledWith('work'));
  });

  it('surfaces an error when the load fails', async () => {
    mockLoad.mockReset();
    mockLoad.mockRejectedValueOnce(new Error('graph unavailable'));
    render(<KnowledgeGapsTab />);
    await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/));
  });
});
