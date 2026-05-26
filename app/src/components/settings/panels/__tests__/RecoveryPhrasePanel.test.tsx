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
    secretStored: true,
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

// Issue #2657: recovery phrase must be hidden by default and require a
// deliberate one-way reveal before the words or the copy action are usable.
describe('RecoveryPhrasePanel — manual reveal gate (#2657)', () => {
  it('blurs the mnemonic grid by default and shows the reveal overlay', () => {
    renderWithProviders(<RecoveryPhrasePanel />);
    const grid = screen.getByTestId('mnemonic-grid');
    expect(grid.className).toContain('blur-md');
    expect(grid.className).toContain('pointer-events-none');
    expect(grid.className).toContain('select-none');
    expect(grid.getAttribute('aria-hidden')).toBe('true');
    // Overlay button is present
    expect(screen.getByRole('button', { name: /reveal phrase/i })).toBeTruthy();
    // Hidden-state copy is announced
    expect(screen.getByText(/recovery phrase is hidden/i)).toBeTruthy();
  });

  it('disables Copy to Clipboard until the phrase is revealed', () => {
    renderWithProviders(<RecoveryPhrasePanel />);
    const copyBtn = screen
      .getAllByRole('button')
      .find(b => /copy to clipboard/i.test(b.textContent || ''));
    expect(copyBtn).toBeTruthy();
    expect(copyBtn!.hasAttribute('disabled')).toBe(true);
    expect(copyBtn!.getAttribute('aria-disabled')).toBe('true');
    expect(copyBtn!.className).toContain('opacity-50');
    expect(copyBtn!.className).toContain('cursor-not-allowed');
  });

  it('reveals the phrase, removes the overlay, and enables Copy on click', () => {
    renderWithProviders(<RecoveryPhrasePanel />);
    const overlay = screen.getByRole('button', { name: /reveal phrase/i });
    fireEvent.click(overlay);

    // Overlay is gone — one-way reveal
    expect(screen.queryByRole('button', { name: /reveal phrase/i })).toBeNull();

    // Grid no longer blurred
    const grid = screen.getByTestId('mnemonic-grid');
    expect(grid.className).not.toContain('blur-md');
    expect(grid.className).not.toContain('pointer-events-none');
    expect(grid.getAttribute('aria-hidden')).toBe('false');

    // Copy is enabled
    const copyBtn = screen
      .getAllByRole('button')
      .find(b => /copy to clipboard/i.test(b.textContent || ''));
    expect(copyBtn).toBeTruthy();
    expect(copyBtn!.hasAttribute('disabled')).toBe(false);
    expect(copyBtn!.getAttribute('aria-disabled')).toBe('false');
  });

  it('writes the mnemonic to clipboard only after reveal', async () => {
    const writeText = vi.fn<(text: string) => Promise<void>>(async () => {
      return;
    });
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
    renderWithProviders(<RecoveryPhrasePanel />);

    fireEvent.click(screen.getByRole('button', { name: /reveal phrase/i }));
    const copyBtn = screen
      .getAllByRole('button')
      .find(b => /copy to clipboard/i.test(b.textContent || ''))!;
    fireEvent.click(copyBtn);

    expect(writeText).toHaveBeenCalledTimes(1);
    const arg = writeText.mock.calls[0]?.[0] ?? '';
    // BIP-39 mnemonics are space-separated words.
    expect(arg.split(' ').length).toBeGreaterThanOrEqual(12);
  });

  it('resets to hidden when switching modes (defense in depth)', () => {
    renderWithProviders(<RecoveryPhrasePanel />);
    // Reveal first
    fireEvent.click(screen.getByRole('button', { name: /reveal phrase/i }));
    expect(screen.queryByRole('button', { name: /reveal phrase/i })).toBeNull();

    // Switch to import then back to generate — a fresh hidden gate must show
    fireEvent.click(screen.getByText(/I already have a recovery phrase/i));
    fireEvent.click(screen.getByText(/Generate a new recovery phrase instead/i));

    // Hidden state restored
    expect(screen.getByRole('button', { name: /reveal phrase/i })).toBeTruthy();
    const grid = screen.getByTestId('mnemonic-grid');
    expect(grid.className).toContain('blur-md');
  });
});
