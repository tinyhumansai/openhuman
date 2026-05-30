import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeEvidenceTrust } from '../../lib/memory/evidenceTrust';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import EvidenceTrustTab from './EvidenceTrustTab';

const mockLoadTrust = vi.fn();
const mockLoadNamespaces = vi.fn();

vi.mock('../../services/api/evidenceTrustApi', () => ({
  loadEvidenceTrust: (...args: unknown[]) => mockLoadTrust(...args),
  loadNamespaces: (...args: unknown[]) => mockLoadNamespaces(...args),
}));

function rel(
  subject: string,
  predicate: string,
  object: string,
  evidenceCount: number
): GraphRelation {
  return {
    namespace: 'n',
    subject,
    predicate,
    object,
    attrs: {},
    updatedAt: 0,
    evidenceCount,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
  };
}

const result = computeEvidenceTrust([rel('A', 'p', 'B', 5), rel('B', 'p', 'C', 2)]);

describe('<EvidenceTrustTab />', () => {
  beforeEach(() => {
    mockLoadTrust.mockReset();
    mockLoadNamespaces.mockReset();
    mockLoadTrust.mockResolvedValue(result);
    mockLoadNamespaces.mockResolvedValue([]);
  });

  it('loads trust (all namespaces) on mount and renders the result', async () => {
    render(<EvidenceTrustTab />);
    expect(mockLoadTrust).toHaveBeenCalledWith(undefined);
    await waitFor(() => expect(screen.getByText('Trust Quotient ranking')).toBeInTheDocument());
  });

  it('shows the namespace selector and re-queries on change', async () => {
    mockLoadNamespaces.mockResolvedValueOnce(['work', 'personal']);
    render(<EvidenceTrustTab />);
    await waitFor(() => screen.getByRole('combobox'));
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'work' } });
    await waitFor(() => expect(mockLoadTrust).toHaveBeenCalledWith('work'));
  });

  it('surfaces an error when the load fails', async () => {
    mockLoadTrust.mockReset();
    mockLoadTrust.mockRejectedValueOnce(new Error('graph unavailable'));
    render(<EvidenceTrustTab />);
    await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/));
  });
});
