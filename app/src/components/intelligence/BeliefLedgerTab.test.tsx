import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands/memory';
import BeliefLedgerTab from './BeliefLedgerTab';

const mockLoadRelations = vi.fn();
const mockExplainConflict = vi.fn();
const mockSaveCorrection = vi.fn();

vi.mock('../../services/api/beliefLedgerApi', () => ({
  loadRelations: (...args: unknown[]) => mockLoadRelations(...args),
  explainConflict: (...args: unknown[]) => mockExplainConflict(...args),
  saveCorrection: (...args: unknown[]) => mockSaveCorrection(...args),
}));

const NOW = 1_700_000_000;
const DAY = 86_400;

function makeRel(overrides: Partial<GraphRelation> = {}): GraphRelation {
  return {
    namespace: 'work',
    subject: 'You',
    predicate: 'lives_in',
    object: 'Berlin',
    attrs: {},
    updatedAt: NOW,
    evidenceCount: 1,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
    ...overrides,
  };
}

const contested: GraphRelation[] = [
  makeRel({ object: 'Berlin', updatedAt: NOW - 120 * DAY, evidenceCount: 3 }),
  makeRel({ object: 'Lisbon', updatedAt: NOW, evidenceCount: 2 }),
];

describe('<BeliefLedgerTab />', () => {
  beforeEach(() => {
    mockLoadRelations.mockReset();
    mockExplainConflict.mockReset();
    mockSaveCorrection.mockReset();
  });

  it('loads relations on mount and renders the claim card', async () => {
    mockLoadRelations.mockResolvedValueOnce(contested);
    render(<BeliefLedgerTab onToast={() => {}} />);
    expect(mockLoadRelations).toHaveBeenCalledTimes(1);
    await waitFor(() => {
      expect(screen.getByRole('article', { name: 'You lives_in' })).toBeInTheDocument();
    });
    expect(screen.getByText('Belief Ledger')).toBeInTheDocument();
  });

  it('surfaces an error alert when loading fails', async () => {
    mockLoadRelations.mockRejectedValueOnce(new Error('graph unavailable'));
    render(<BeliefLedgerTab onToast={() => {}} />);
    await waitFor(() => {
      expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/);
    });
  });

  it('round-trips an explanation: click Explain -> renders the returned text', async () => {
    mockLoadRelations.mockResolvedValueOnce(contested);
    mockExplainConflict.mockResolvedValueOnce('They relocated from Berlin to Lisbon.');
    render(<BeliefLedgerTab onToast={() => {}} />);
    await waitFor(() => screen.getByRole('article', { name: 'You lives_in' }));
    fireEvent.click(screen.getByRole('button', { name: /Explain this contradiction/ }));
    await waitFor(() => {
      expect(screen.getByText('They relocated from Berlin to Lisbon.')).toBeInTheDocument();
    });
    expect(mockExplainConflict).toHaveBeenCalledTimes(1);
  });

  it('round-trips a correction: Set the truth -> Confirm -> saveCorrection + reload + success toast', async () => {
    mockLoadRelations.mockResolvedValue(contested);
    mockSaveCorrection.mockResolvedValueOnce(undefined);
    const onToast = vi.fn();
    render(<BeliefLedgerTab onToast={onToast} />);
    await waitFor(() => screen.getByRole('article', { name: 'You lives_in' }));

    fireEvent.click(screen.getByRole('button', { name: /Set the truth/ }));
    // ConfirmationModal is now open.
    expect(screen.getByText('Set the correct value')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Confirm' }));

    await waitFor(() => {
      expect(mockSaveCorrection).toHaveBeenCalledTimes(1);
    });
    expect(mockSaveCorrection).toHaveBeenCalledWith({
      namespace: 'work',
      subject: 'You',
      predicate: 'lives_in',
      correctObject: 'Lisbon',
    });
    // Reloaded after saving (mount + post-save).
    await waitFor(() => expect(mockLoadRelations).toHaveBeenCalledTimes(2));
    expect(onToast).toHaveBeenCalledWith(expect.objectContaining({ type: 'success' }));
  });

  it('shows an error toast if saving the correction fails', async () => {
    mockLoadRelations.mockResolvedValue(contested);
    mockSaveCorrection.mockRejectedValueOnce(new Error('ingest failed'));
    const onToast = vi.fn();
    render(<BeliefLedgerTab onToast={onToast} />);
    await waitFor(() => screen.getByRole('article', { name: 'You lives_in' }));
    fireEvent.click(screen.getByRole('button', { name: /Set the truth/ }));
    fireEvent.click(screen.getByRole('button', { name: 'Confirm' }));
    await waitFor(() => {
      expect(onToast).toHaveBeenCalledWith(expect.objectContaining({ type: 'error' }));
    });
  });
});
