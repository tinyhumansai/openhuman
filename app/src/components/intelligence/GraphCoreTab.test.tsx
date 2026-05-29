import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeGraphCore } from '../../lib/memory/graphCore';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import GraphCoreTab from './GraphCoreTab';

const mockLoadCore = vi.fn();
const mockLoadNamespaces = vi.fn();

vi.mock('../../services/api/graphCoreApi', () => ({
  loadCore: (...args: unknown[]) => mockLoadCore(...args),
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

const result = computeGraphCore([rel('A', 'B'), rel('B', 'C'), rel('C', 'A')]);

describe('<GraphCoreTab />', () => {
  beforeEach(() => {
    mockLoadCore.mockReset();
    mockLoadNamespaces.mockReset();
    mockLoadCore.mockResolvedValue(result);
    mockLoadNamespaces.mockResolvedValue([]);
  });

  it('loads core (all namespaces) on mount and renders the result', async () => {
    render(<GraphCoreTab />);
    expect(mockLoadCore).toHaveBeenCalledWith(undefined);
    await waitFor(() => expect(screen.getByText('Deepest-core entities')).toBeInTheDocument());
  });

  it('shows the namespace selector and re-queries on change', async () => {
    mockLoadNamespaces.mockResolvedValueOnce(['work', 'personal']);
    render(<GraphCoreTab />);
    await waitFor(() => screen.getByRole('combobox'));
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'work' } });
    await waitFor(() => expect(mockLoadCore).toHaveBeenCalledWith('work'));
  });

  it('surfaces an error when the load fails', async () => {
    mockLoadCore.mockReset();
    mockLoadCore.mockRejectedValueOnce(new Error('graph unavailable'));
    render(<GraphCoreTab />);
    await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/));
  });
});
