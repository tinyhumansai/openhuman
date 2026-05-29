import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeDocumentProvenance } from '../../lib/memory/documentProvenance';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import DocumentProvenanceTab from './DocumentProvenanceTab';

const mockLoadProvenance = vi.fn();
const mockLoadNamespaces = vi.fn();

vi.mock('../../services/api/documentProvenanceApi', () => ({
  loadProvenance: (...args: unknown[]) => mockLoadProvenance(...args),
  loadNamespaces: (...args: unknown[]) => mockLoadNamespaces(...args),
}));

function rel(subject: string, predicate: string, object: string, docs: string[]): GraphRelation {
  return {
    namespace: 'n',
    subject,
    predicate,
    object,
    attrs: {},
    updatedAt: 0,
    evidenceCount: 1,
    orderIndex: null,
    documentIds: docs,
    chunkIds: [],
  };
}

const result = computeDocumentProvenance([
  rel('A', 'knows', 'B', ['d1']),
  rel('A', 'knows', 'C', ['d1', 'd2']),
]);

describe('<DocumentProvenanceTab />', () => {
  beforeEach(() => {
    mockLoadProvenance.mockReset();
    mockLoadNamespaces.mockReset();
    mockLoadProvenance.mockResolvedValue(result);
    mockLoadNamespaces.mockResolvedValue([]);
  });

  it('loads provenance (all namespaces) on mount and renders the result', async () => {
    render(<DocumentProvenanceTab />);
    expect(mockLoadProvenance).toHaveBeenCalledWith(undefined);
    await waitFor(() => expect(screen.getByText('Per-document footprint')).toBeInTheDocument());
  });

  it('shows the namespace selector and re-queries on change', async () => {
    mockLoadNamespaces.mockResolvedValueOnce(['work', 'personal']);
    render(<DocumentProvenanceTab />);
    await waitFor(() => screen.getByRole('combobox'));
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'work' } });
    await waitFor(() => expect(mockLoadProvenance).toHaveBeenCalledWith('work'));
  });

  it('surfaces an error when the load fails', async () => {
    mockLoadProvenance.mockReset();
    mockLoadProvenance.mockRejectedValueOnce(new Error('graph unavailable'));
    render(<DocumentProvenanceTab />);
    await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/));
  });
});
