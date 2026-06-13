import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands/memory';
import ConnectionPathTab from './ConnectionPathTab';

const mockLoadGraph = vi.fn();
const mockLoadNamespaces = vi.fn();

vi.mock('../../services/api/connectionPathApi', () => ({
  loadGraph: (...args: unknown[]) => mockLoadGraph(...args),
  loadNamespaces: (...args: unknown[]) => mockLoadNamespaces(...args),
}));

function rel(subject: string, object: string, predicate = 'p'): GraphRelation {
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

const graph = {
  entities: ['A', 'B', 'C'],
  relations: [rel('A', 'B', 'knows'), rel('B', 'C', 'likes')],
};

describe('<ConnectionPathTab />', () => {
  beforeEach(() => {
    mockLoadGraph.mockReset();
    mockLoadNamespaces.mockReset();
    mockLoadGraph.mockResolvedValue(graph);
    mockLoadNamespaces.mockResolvedValue([]);
  });

  it('loads the graph on mount and prompts for endpoints', async () => {
    render(<ConnectionPathTab />);
    expect(mockLoadGraph).toHaveBeenCalledWith(undefined);
    await waitFor(() =>
      expect(
        screen.getByText('Pick two entities to trace how they are connected.')
      ).toBeInTheDocument()
    );
  });

  it('traces the shortest path once both endpoints are entered', async () => {
    render(<ConnectionPathTab />);
    await waitFor(() => screen.getByText('Pick two entities to trace how they are connected.'));
    fireEvent.change(screen.getByLabelText('From'), { target: { value: 'A' } });
    fireEvent.change(screen.getByLabelText('To'), { target: { value: 'C' } });
    await waitFor(() => expect(screen.getByText('Shortest path')).toBeInTheDocument());
    expect(screen.getByText('knows →')).toBeInTheDocument();
    expect(screen.getByText('likes →')).toBeInTheDocument();
  });

  it('surfaces an error when the graph fails to load', async () => {
    mockLoadGraph.mockReset();
    mockLoadGraph.mockRejectedValueOnce(new Error('graph unavailable'));
    render(<ConnectionPathTab />);
    await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/));
  });
});
