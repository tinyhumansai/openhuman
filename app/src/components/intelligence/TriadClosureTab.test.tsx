import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeTriadClosure } from '../../lib/memory/triadClosure';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import TriadClosureTab from './TriadClosureTab';

const mockLoadClosure = vi.fn();
const mockLoadNamespaces = vi.fn();

vi.mock('../../services/api/triadClosureApi', () => ({
  loadTriadClosure: (...args: unknown[]) => mockLoadClosure(...args),
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

const result = computeTriadClosure([rel('A', 'B'), rel('B', 'C'), rel('A', 'D'), rel('D', 'C')]);

describe('<TriadClosureTab />', () => {
  beforeEach(() => {
    mockLoadClosure.mockReset();
    mockLoadNamespaces.mockReset();
    mockLoadClosure.mockResolvedValue(result);
    mockLoadNamespaces.mockResolvedValue([]);
  });

  it('loads hints (all namespaces) on mount and renders the result', async () => {
    render(<TriadClosureTab />);
    expect(mockLoadClosure).toHaveBeenCalledWith(undefined);
    await waitFor(() =>
      expect(screen.getByText('Suggested edges to consider')).toBeInTheDocument()
    );
  });

  it('shows the namespace selector and re-queries on change', async () => {
    mockLoadNamespaces.mockResolvedValueOnce(['work', 'personal']);
    render(<TriadClosureTab />);
    await waitFor(() => screen.getByRole('combobox'));
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'work' } });
    await waitFor(() => expect(mockLoadClosure).toHaveBeenCalledWith('work'));
  });

  it('surfaces an error when the load fails', async () => {
    mockLoadClosure.mockReset();
    mockLoadClosure.mockRejectedValueOnce(new Error('graph unavailable'));
    render(<TriadClosureTab />);
    await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/));
  });
});
