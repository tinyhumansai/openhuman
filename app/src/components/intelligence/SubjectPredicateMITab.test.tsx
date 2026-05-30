import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeSubjectPredicateMI } from '../../lib/memory/subjectPredicateMI';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import SubjectPredicateMITab from './SubjectPredicateMITab';

const mockLoadMI = vi.fn();
const mockLoadNamespaces = vi.fn();

vi.mock('../../services/api/subjectPredicateMIApi', () => ({
  loadSubjectPredicateMI: (...args: unknown[]) => mockLoadMI(...args),
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

const result = computeSubjectPredicateMI([rel('A', 'knows', 'X'), rel('B', 'trusts', 'X')]);

describe('<SubjectPredicateMITab />', () => {
  beforeEach(() => {
    mockLoadMI.mockReset();
    mockLoadNamespaces.mockReset();
    mockLoadMI.mockResolvedValue(result);
    mockLoadNamespaces.mockResolvedValue([]);
  });

  it('loads MI (all namespaces) on mount and renders the result', async () => {
    render(<SubjectPredicateMITab />);
    expect(mockLoadMI).toHaveBeenCalledWith(undefined);
    await waitFor(() =>
      expect(screen.getByText('Most-to-least specialised subjects')).toBeInTheDocument()
    );
  });

  it('shows the namespace selector and re-queries on change', async () => {
    mockLoadNamespaces.mockResolvedValueOnce(['work', 'personal']);
    render(<SubjectPredicateMITab />);
    await waitFor(() => screen.getByRole('combobox'));
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'work' } });
    await waitFor(() => expect(mockLoadMI).toHaveBeenCalledWith('work'));
  });

  it('surfaces an error when the load fails', async () => {
    mockLoadMI.mockReset();
    mockLoadMI.mockRejectedValueOnce(new Error('graph unavailable'));
    render(<SubjectPredicateMITab />);
    await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/));
  });
});
