import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeEntityAssociations } from '../../lib/memory/entityAssociations';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import EntityAssociationsTab from './EntityAssociationsTab';

const mockLoadAssociations = vi.fn();
const mockLoadNamespaces = vi.fn();

vi.mock('../../services/api/entityAssociationsApi', () => ({
  loadAssociations: (...args: unknown[]) => mockLoadAssociations(...args),
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

const report = computeEntityAssociations([
  rel('X', 'A'),
  rel('X', 'B'),
  rel('Y', 'A'),
  rel('Y', 'B'),
]);

describe('<EntityAssociationsTab />', () => {
  beforeEach(() => {
    mockLoadAssociations.mockReset();
    mockLoadNamespaces.mockReset();
    mockLoadAssociations.mockResolvedValue(report);
    mockLoadNamespaces.mockResolvedValue([]);
  });

  it('loads associations (all namespaces) on mount and renders the result', async () => {
    render(<EntityAssociationsTab />);
    expect(mockLoadAssociations).toHaveBeenCalledWith(undefined);
    await waitFor(() => expect(screen.getByText('Strongest associations')).toBeInTheDocument());
  });

  it('shows the namespace selector and re-queries on change', async () => {
    mockLoadNamespaces.mockResolvedValueOnce(['work', 'personal']);
    render(<EntityAssociationsTab />);
    await waitFor(() => screen.getByRole('combobox'));
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'work' } });
    await waitFor(() => expect(mockLoadAssociations).toHaveBeenCalledWith('work'));
  });

  it('surfaces an error when the load fails', async () => {
    mockLoadAssociations.mockReset();
    mockLoadAssociations.mockRejectedValueOnce(new Error('graph unavailable'));
    render(<EntityAssociationsTab />);
    await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/));
  });
});
