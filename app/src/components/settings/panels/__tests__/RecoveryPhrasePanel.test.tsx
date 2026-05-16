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

vi.mock('../../../../services/walletApi', () => ({
  setupLocalWallet: vi.fn(async () => ({
    configured: true,
    onboardingCompleted: true,
    consentGranted: true,
    source: 'generated',
    mnemonicWordCount: 12,
    accounts: [],
    updatedAtMs: Date.now(),
  })),
}));

describe('RecoveryPhrasePanel — trust-surface polish', () => {
  it('renders the amber warning callout in generate mode', () => {
    const { container } = renderWithProviders(<RecoveryPhrasePanel />);
    expect(screen.getByText(/can never be recovered if lost/i)).toBeTruthy();
    // Polish guarantee: the disclaimer lives in its own amber callout,
    // not buried in body text.
    expect(container.querySelector('.bg-amber-50')).not.toBeNull();
  });

  it('renders import-mode intro copy when switching modes', () => {
    renderWithProviders(<RecoveryPhrasePanel />);
    fireEvent.click(screen.getByText(/I already have a recovery phrase/i));
    expect(screen.getByText(/Enter your recovery phrase below/i)).toBeTruthy();
  });

  it('uses palette token text-stone-700 on the confirm-checkbox label (not opacity)', () => {
    const { container } = renderWithProviders(<RecoveryPhrasePanel />);
    const label = screen.getByText(/consent to using it for local wallet setup/i);
    expect(label.className).toContain('text-stone-700');
    // Sanity: the old opacity hack is gone from this label.
    expect(label.className).not.toContain('opacity-80');
    expect(container).toBeTruthy();
  });
});

// Batch-5: recovery/mnemonic mode-switch state reset (pr#1646)
describe('RecoveryPhrasePanel — mode-switch state reset', () => {
  it('switches to import mode and shows import-mode UI', () => {
    renderWithProviders(<RecoveryPhrasePanel />);
    // Default: generate mode — amber callout visible
    expect(screen.getByText(/can never be recovered if lost/i)).toBeTruthy();

    // Switch to import mode
    fireEvent.click(screen.getByText(/I already have a recovery phrase/i));
    expect(screen.getByText(/Enter your recovery phrase below/i)).toBeTruthy();
  });

  it('resets confirmed checkbox when switching from generate to import', () => {
    renderWithProviders(<RecoveryPhrasePanel />);

    // Check the confirmed checkbox in generate mode
    const checkbox = screen.getByRole('checkbox');
    fireEvent.click(checkbox);
    expect(checkbox).toBeChecked();

    // Switch to import mode — confirmed should reset
    fireEvent.click(screen.getByText(/I already have a recovery phrase/i));
    // In import mode the "consent" checkbox is not shown, so confirmed state is reset
    expect(screen.queryByRole('checkbox')).toBeNull();

    // Switch back to generate — checkbox should be unchecked (reset to false)
    fireEvent.click(screen.getByText(/Generate a new recovery phrase instead/i));
    const regeneratedCheckbox = screen.getByRole('checkbox');
    expect(regeneratedCheckbox).not.toBeChecked();
  });

  it('shows generate-mode UI again after switching back from import', () => {
    renderWithProviders(<RecoveryPhrasePanel />);
    fireEvent.click(screen.getByText(/I already have a recovery phrase/i));
    expect(screen.getByText(/Enter your recovery phrase below/i)).toBeTruthy();

    fireEvent.click(screen.getByText(/Generate a new recovery phrase instead/i));
    // Back in generate mode
    expect(screen.getByText(/can never be recovered if lost/i)).toBeTruthy();
  });
});
