import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import RecoveryPhrasePanel from '../RecoveryPhrasePanel';

vi.mock('../../../../providers/CoreStateProvider', () => ({
  useCoreState: () => ({
    snapshot: { currentUser: null },
    setEncryptionKey: vi.fn(async () => undefined),
  }),
}));

describe('RecoveryPhrasePanel — trust-surface polish', () => {
  it('renders the amber warning callout in generate mode', () => {
    const { container } = renderWithProviders(<RecoveryPhrasePanel />);
    expect(screen.getByText(/can never be recovered if lost/i)).toBeInTheDocument();
    // Polish guarantee: the disclaimer lives in its own amber callout,
    // not buried in body text.
    expect(container.querySelector('.bg-amber-50')).not.toBeNull();
  });

  it('renders import-mode intro copy when switching modes', () => {
    renderWithProviders(<RecoveryPhrasePanel />);
    fireEvent.click(screen.getByText(/I already have a recovery phrase/i));
    expect(screen.getByText(/Enter your recovery phrase below/i)).toBeInTheDocument();
  });

  it('uses palette token text-stone-700 on the confirm-checkbox label (not opacity)', () => {
    const { container } = renderWithProviders(<RecoveryPhrasePanel />);
    const label = screen.getByText(/I have saved my recovery phrase in a safe place/i);
    expect(label.className).toContain('text-stone-700');
    // Sanity: the old opacity hack is gone from this label.
    expect(label.className).not.toContain('opacity-80');
    expect(container).toBeTruthy();
  });
});
