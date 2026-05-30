import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeNamespaceOverview } from '../../lib/memory/namespaceOverview';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import NamespaceOverviewTab from './NamespaceOverviewTab';

const mockLoad = vi.fn();

vi.mock('../../services/api/namespaceOverviewApi', () => ({
  loadNamespaceOverview: (...args: unknown[]) => mockLoad(...args),
}));

function rel(namespace: string | null, subject: string, object: string): GraphRelation {
  return {
    namespace,
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

const report = computeNamespaceOverview([rel('work', 'A', 'B')]);

describe('<NamespaceOverviewTab />', () => {
  beforeEach(() => {
    mockLoad.mockReset();
    mockLoad.mockResolvedValue(report);
  });

  it('loads on mount and renders the per-namespace list', async () => {
    render(<NamespaceOverviewTab />);
    await waitFor(() => expect(screen.getByText('By namespace')).toBeInTheDocument());
    expect(mockLoad).toHaveBeenCalledTimes(1);
  });

  it('surfaces an error when the load fails', async () => {
    mockLoad.mockReset();
    mockLoad.mockRejectedValueOnce(new Error('graph unavailable'));
    render(<NamespaceOverviewTab />);
    await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/));
  });
});
