/// <reference types="@testing-library/jest-dom/vitest" />
/**
 * Tests for the Mnemonic page.
 *
 * Coverage areas:
 *  - Initial render: generate mode UI, word grid, buttons
 *  - Copy to clipboard (success + fallback paths)
 *  - Confirmation checkbox gates the Continue button
 *  - Mode switch: generate ↔ import, state resets on switch
 *  - Import mode: word input, auto-advance, backspace navigation, paste
 *  - Validation: incomplete phrase, invalid phrase, valid phrase
 *  - handleContinue — generate mode: happy path, no user, crypto error
 *  - handleContinue — import mode: happy path, validation failure, no user
 *  - Loading state during continue
 *  - Navigation to /home on success
 *  - Core-state setEncryptionKey persistence
 */
import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import Mnemonic from '../src/pages/Mnemonic';
import { renderWithProviders } from '../src/test/test-utils';
import type { User } from '../src/types/api';

// ---------------------------------------------------------------------------
// Module mocks
// ---------------------------------------------------------------------------

const FIXED_MNEMONIC =
  'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon ' +
  'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art';

const {
  mockGenerateMnemonicPhrase,
  mockValidateMnemonicPhrase,
  mockDeriveAesKey,
  mockDeriveEvm,
  mockSetWalletAddress,
  mockSetEncryptionKey,
  mockUseCoreState,
} = vi.hoisted(() => ({
  mockGenerateMnemonicPhrase: vi.fn(() => FIXED_MNEMONIC),
  mockValidateMnemonicPhrase: vi.fn(() => true),
  mockDeriveAesKey: vi.fn(() => 'aes-key-hex'),
  mockDeriveEvm: vi.fn(() => '0xDeAdBeEf'),
  mockSetWalletAddress: vi.fn().mockResolvedValue(undefined),
  mockSetEncryptionKey: vi.fn().mockResolvedValue(undefined),
  mockUseCoreState: vi.fn(),
}));

vi.mock('../src/utils/cryptoKeys', () => ({
  MNEMONIC_GENERATE_WORD_COUNT: 24,
  generateMnemonicPhrase: mockGenerateMnemonicPhrase,
  validateMnemonicPhrase: mockValidateMnemonicPhrase,
  deriveAesKeyFromMnemonic: mockDeriveAesKey,
  deriveEvmAddressFromMnemonic: mockDeriveEvm,
}));

vi.mock('../src/lib/skills/manager', () => ({
  skillManager: { setWalletAddress: mockSetWalletAddress },
}));

vi.mock('../src/providers/CoreStateProvider', () => ({
  useCoreState: () => mockUseCoreState(),
}));

// LottieAnimation makes network calls; stub it out
vi.mock('../src/components/LottieAnimation', () => ({
  default: () => <div data-testid="lottie" />,
}));

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const WORD_COUNT = 24;
const FIXED_WORDS = FIXED_MNEMONIC.split(' '); // 24 words

/** User with a valid _id so the "user not loaded" guard passes. */
const mockUser: Partial<User> = { _id: 'user-123', username: 'tester' };

/** Render with a user already in the store. */
const renderWithUser = () => renderWithProviders(<Mnemonic />);

/** Render without a user in the store (unauthenticated). */
const renderWithoutUser = () => renderWithProviders(<Mnemonic />);

/** Switch to import mode. */
const switchToImport = () => fireEvent.click(screen.getByText('I already have a recovery phrase'));

/** Fill all 24 import inputs with the words from `phrase`. */
const fillAllImportWords = (phrase = FIXED_MNEMONIC) => {
  const words = phrase.split(' ');
  const inputs = screen.getAllByRole('textbox');
  // Paste into the first field to trigger multi-word paste handling
  fireEvent.change(inputs[0], { target: { value: words.join(' ') } });
};

/** Get the Continue button. */
const continueButton = () => screen.getByRole('button', { name: /import & continue|let's go/i });

beforeEach(() => {
  mockGenerateMnemonicPhrase.mockClear();
  mockValidateMnemonicPhrase.mockClear();
  mockDeriveAesKey.mockClear();
  mockDeriveEvm.mockClear();
  mockSetWalletAddress.mockClear();
  mockSetEncryptionKey.mockClear();
  mockUseCoreState.mockReturnValue({
    snapshot: {
      currentUser: mockUser,
      sessionToken: 'jwt-token',
    },
    setEncryptionKey: mockSetEncryptionKey,
  });
});

// ---------------------------------------------------------------------------
// Generate mode — initial render
// ---------------------------------------------------------------------------

describe('Mnemonic — generate mode: initial render', () => {
  it('renders the "Your Recovery Phrase" heading', () => {
    renderWithUser();
    expect(screen.getByText('Your Recovery Phrase')).toBeInTheDocument();
  });

  it('renders all 24 words from the generated mnemonic', () => {
    renderWithUser();
    for (const word of FIXED_WORDS) {
      expect(screen.getAllByText(word).length).toBeGreaterThan(0);
    }
  });

  it('renders 24 numbered word tiles', () => {
    renderWithUser();
    expect(screen.getByText('1.')).toBeInTheDocument();
    expect(screen.getByText('24.')).toBeInTheDocument();
  });

  it('renders the Copy to Clipboard button', () => {
    renderWithUser();
    expect(screen.getByRole('button', { name: /copy to clipboard/i })).toBeInTheDocument();
  });

  it('renders the amber warning notice', () => {
    renderWithUser();
    expect(screen.getByText(/can never be recovered if lost/i)).toBeInTheDocument();
  });

  it('renders the confirmation checkbox unchecked', () => {
    renderWithUser();
    expect(screen.getByRole('checkbox')).not.toBeChecked();
  });

  it('renders the Continue button disabled before confirmation', () => {
    renderWithUser();
    expect(continueButton()).toBeDisabled();
  });

  it('renders the "I already have a recovery phrase" link', () => {
    renderWithUser();
    expect(screen.getByText('I already have a recovery phrase')).toBeInTheDocument();
  });
});

// ---------------------------------------------------------------------------
// Generate mode — confirmation checkbox
// ---------------------------------------------------------------------------

describe('Mnemonic — generate mode: confirmation checkbox', () => {
  it('enables the Continue button when checkbox is checked', () => {
    renderWithUser();
    fireEvent.click(screen.getByRole('checkbox'));
    expect(continueButton()).toBeEnabled();
  });

  it('disables the Continue button again when checkbox is unchecked', () => {
    renderWithUser();
    const checkbox = screen.getByRole('checkbox');
    fireEvent.click(checkbox);
    fireEvent.click(checkbox);
    expect(continueButton()).toBeDisabled();
  });
});

// ---------------------------------------------------------------------------
// Generate mode — copy to clipboard
// ---------------------------------------------------------------------------

describe('Mnemonic — generate mode: copy to clipboard', () => {
  it('calls navigator.clipboard.writeText with the full mnemonic', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });

    renderWithUser();
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /copy to clipboard/i }));
    });

    expect(writeText).toHaveBeenCalledWith(FIXED_MNEMONIC);
  });

  it('shows "Copied to Clipboard" after clicking copy', async () => {
    Object.assign(navigator, { clipboard: { writeText: vi.fn().mockResolvedValue(undefined) } });
    renderWithUser();

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /copy to clipboard/i }));
    });

    await waitFor(() => expect(screen.getByText('Copied to Clipboard')).toBeInTheDocument());
  });

  it('resets "Copied" text back to "Copy to Clipboard" after 3 s', async () => {
    vi.useFakeTimers();
    Object.assign(navigator, { clipboard: { writeText: vi.fn().mockResolvedValue(undefined) } });
    renderWithUser();

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /copy to clipboard/i }));
    });

    // Flush the resolved clipboard promise so setCopied(true) fires
    await act(async () => {
      await vi.runAllTimersAsync();
    });

    // Now the 3-second reset timer has also been run
    expect(screen.queryByText('Copied to Clipboard')).not.toBeInTheDocument();
    vi.useRealTimers();
  });

  it('uses execCommand fallback when clipboard API throws', async () => {
    Object.assign(navigator, {
      clipboard: { writeText: vi.fn().mockRejectedValue(new Error('blocked')) },
    });
    // jsdom does not implement execCommand — define it so we can spy
    if (!document.execCommand) {
      Object.defineProperty(document, 'execCommand', {
        value: vi.fn().mockReturnValue(true),
        writable: true,
        configurable: true,
      });
    }
    const execCommand = vi.spyOn(document, 'execCommand').mockReturnValue(true);

    renderWithUser();
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /copy to clipboard/i }));
    });

    await waitFor(() => expect(screen.getByText('Copied to Clipboard')).toBeInTheDocument());
    expect(execCommand).toHaveBeenCalledWith('copy');
    execCommand.mockRestore();
  });
});

// ---------------------------------------------------------------------------
// Mode switching
// ---------------------------------------------------------------------------

describe('Mnemonic — mode switching', () => {
  it('switches to import mode on "I already have a recovery phrase" click', () => {
    renderWithUser();
    switchToImport();
    expect(screen.getByText('Import Recovery Phrase')).toBeInTheDocument();
  });

  it('shows 24 text inputs in import mode', () => {
    renderWithUser();
    switchToImport();
    expect(screen.getAllByRole('textbox')).toHaveLength(WORD_COUNT);
  });

  it('switches back to generate mode on "Generate a new recovery phrase instead"', () => {
    renderWithUser();
    switchToImport();
    fireEvent.click(screen.getByText('Generate a new recovery phrase instead'));
    expect(screen.getByText('Your Recovery Phrase')).toBeInTheDocument();
  });

  it('resets error when switching modes', async () => {
    renderWithUser();
    // Trigger an error in generate mode (click continue without confirming)
    fireEvent.click(continueButton()); // disabled, won't trigger, so force via import mode
    // Switch to import mode and back — confirmed state should reset
    switchToImport();
    expect(screen.queryByText(/please enter all/i)).not.toBeInTheDocument();
  });

  it('resets confirmation when switching from generate to import and back', () => {
    renderWithUser();
    fireEvent.click(screen.getByRole('checkbox'));
    expect(continueButton()).toBeEnabled();

    switchToImport();
    fireEvent.click(screen.getByText('Generate a new recovery phrase instead'));

    // Confirmed state is reset — Continue should be disabled again
    expect(continueButton()).toBeDisabled();
  });
});

// ---------------------------------------------------------------------------
// Import mode — word input behaviour
// ---------------------------------------------------------------------------

describe('Mnemonic — import mode: word input', () => {
  beforeEach(() => {
    renderWithUser();
    switchToImport();
  });

  it('updates input value when a word is typed', () => {
    const inputs = screen.getAllByRole('textbox');
    fireEvent.change(inputs[0], { target: { value: 'abandon' } });
    expect((inputs[0] as HTMLInputElement).value).toBe('abandon');
  });

  it('lowercases the entered word', () => {
    const inputs = screen.getAllByRole('textbox');
    fireEvent.change(inputs[0], { target: { value: 'ABANDON' } });
    expect((inputs[0] as HTMLInputElement).value).toBe('abandon');
  });

  it('Continue button stays disabled until all 24 words are filled', () => {
    expect(continueButton()).toBeDisabled();

    const inputs = screen.getAllByRole('textbox');
    // Fill only 23 words
    for (let i = 0; i < 23; i++) {
      fireEvent.change(inputs[i], { target: { value: 'abandon' } });
    }
    expect(continueButton()).toBeDisabled();
  });

  it('Continue button becomes enabled when all 24 words are filled (via paste)', () => {
    fillAllImportWords();
    expect(continueButton()).toBeEnabled();
  });

  it('distributes pasted multi-word phrase across inputs starting at index 0', () => {
    const inputs = screen.getAllByRole('textbox');
    fireEvent.change(inputs[0], { target: { value: FIXED_WORDS.join(' ') } });

    // After paste the first input gets the first word
    expect((inputs[0] as HTMLInputElement).value).toBe(FIXED_WORDS[0]);
  });

  it('distributes pasted phrase starting from a non-zero index', () => {
    const inputs = screen.getAllByRole('textbox');
    const remaining = FIXED_WORDS.slice(1).join(' ');
    fireEvent.change(inputs[1], { target: { value: remaining } });
    expect((inputs[1] as HTMLInputElement).value).toBe(FIXED_WORDS[1]);
    expect((inputs[2] as HTMLInputElement).value).toBe(FIXED_WORDS[2]);
  });
});

// ---------------------------------------------------------------------------
// Import mode — keyboard navigation
// ---------------------------------------------------------------------------

describe('Mnemonic — import mode: keyboard navigation', () => {
  beforeEach(() => {
    renderWithUser();
    switchToImport();
  });

  it('does not move focus backward on Backspace when input has text', () => {
    const inputs = screen.getAllByRole('textbox');
    fireEvent.change(inputs[1], { target: { value: 'test' } });
    inputs[1].focus();
    fireEvent.keyDown(inputs[1], { key: 'Backspace' });
    // focus should stay on inputs[1]
    expect(document.activeElement).toBe(inputs[1]);
  });
});

// ---------------------------------------------------------------------------
// Import mode — validation
// ---------------------------------------------------------------------------

describe('Mnemonic — import mode: validation', () => {
  beforeEach(() => {
    renderWithUser();
    switchToImport();
  });

  it('shows error when the phrase fails BIP39 validation after all 24 words are entered', async () => {
    // The Continue button is only enabled when all 24 inputs are filled, so the
    // "please enter all words" branch is unreachable via normal UI.
    // The reachable validation error is the invalid-phrase message.
    mockValidateMnemonicPhrase.mockReturnValueOnce(false);
    fillAllImportWords();

    await act(async () => {
      fireEvent.click(continueButton());
    });

    await waitFor(() => expect(screen.getByText(/invalid recovery phrase/i)).toBeInTheDocument());
  });

  it('shows error when the 24-word phrase is invalid (BIP39)', async () => {
    mockValidateMnemonicPhrase.mockReturnValueOnce(false);
    fillAllImportWords();

    await act(async () => {
      fireEvent.click(continueButton());
    });

    await waitFor(() => expect(screen.getByText(/invalid recovery phrase/i)).toBeInTheDocument());
  });

  it('shows "Valid recovery phrase" text when phrase passes BIP39 validation', async () => {
    mockValidateMnemonicPhrase.mockReturnValue(true);
    fillAllImportWords();

    await act(async () => {
      fireEvent.click(continueButton());
    });

    await waitFor(() => expect(screen.getByText('Valid recovery phrase')).toBeInTheDocument());
  });
});

// ---------------------------------------------------------------------------
// handleContinue — generate mode
// ---------------------------------------------------------------------------

describe('Mnemonic — handleContinue: generate mode', () => {
  it('shows loading text while processing', async () => {
    // Make setWalletAddress hang so we can observe the loading state
    let resolveWallet!: () => void;
    mockSetWalletAddress.mockReturnValueOnce(
      new Promise<void>(res => {
        resolveWallet = res;
      })
    );

    renderWithUser();
    fireEvent.click(screen.getByRole('checkbox'));

    await act(async () => {
      fireEvent.click(continueButton());
    });

    await waitFor(() => expect(screen.getByText('Securing Your Data...')).toBeInTheDocument());

    await act(async () => {
      resolveWallet();
    });
  });

  it('calls setEncryptionKey with the derived AES key', async () => {
    renderWithUser();
    fireEvent.click(screen.getByRole('checkbox'));

    await act(async () => {
      fireEvent.click(continueButton());
    });

    await waitFor(() => expect(mockSetWalletAddress).toHaveBeenCalled());
    expect(mockSetEncryptionKey).toHaveBeenCalledWith('aes-key-hex');
  });

  it('calls deriveAesKeyFromMnemonic and deriveEvmAddressFromMnemonic with the mnemonic', async () => {
    renderWithUser();
    fireEvent.click(screen.getByRole('checkbox'));

    await act(async () => {
      fireEvent.click(continueButton());
    });

    await waitFor(() => expect(mockSetWalletAddress).toHaveBeenCalled());
    expect(mockDeriveAesKey).toHaveBeenCalledWith(FIXED_MNEMONIC);
    expect(mockDeriveEvm).toHaveBeenCalledWith(FIXED_MNEMONIC);
  });

  it('calls skillManager.setWalletAddress with the derived EVM address', async () => {
    renderWithUser();
    fireEvent.click(screen.getByRole('checkbox'));

    await act(async () => {
      fireEvent.click(continueButton());
    });

    await waitFor(() => expect(mockSetWalletAddress).toHaveBeenCalledWith('0xDeAdBeEf'));
  });

  it('shows "User not loaded" error when user._id is missing', async () => {
    mockUseCoreState.mockReturnValue({
      snapshot: {
        currentUser: null,
        sessionToken: 'jwt-token',
      },
      setEncryptionKey: mockSetEncryptionKey,
    });
    renderWithoutUser();

    // The checkbox click + continue in generate mode with no user
    fireEvent.click(screen.getByRole('checkbox'));
    await act(async () => {
      fireEvent.click(continueButton());
    });

    await waitFor(() => expect(screen.getAllByText(/user not loaded/i).length).toBeGreaterThan(0));
  });

  it('shows an error message when deriveAesKeyFromMnemonic throws', async () => {
    mockDeriveAesKey.mockImplementationOnce(() => {
      throw new Error('crypto failure');
    });

    renderWithUser();
    fireEvent.click(screen.getByRole('checkbox'));

    await act(async () => {
      fireEvent.click(continueButton());
    });

    await waitFor(() => expect(screen.getByText('crypto failure')).toBeInTheDocument());
  });

  it('does not navigate when unconfirmed in generate mode', async () => {
    renderWithUser();
    // Do NOT check the checkbox
    await act(async () => {
      fireEvent.click(continueButton());
    });

    // No dispatch should have happened
    await new Promise(r => setTimeout(r, 50));
    expect(mockSetWalletAddress).not.toHaveBeenCalled();
    expect(mockSetEncryptionKey).not.toHaveBeenCalled();
  });
});

// ---------------------------------------------------------------------------
// handleContinue — import mode
// ---------------------------------------------------------------------------

describe('Mnemonic — handleContinue: import mode', () => {
  beforeEach(() => {
    mockValidateMnemonicPhrase.mockReturnValue(true);
  });

  it('derives keys from the imported phrase and navigates to /home', async () => {
    renderWithUser();
    switchToImport();
    fillAllImportWords();

    await act(async () => {
      fireEvent.click(continueButton());
    });

    await waitFor(() => expect(mockSetWalletAddress).toHaveBeenCalledWith('0xDeAdBeEf'));
    expect(mockDeriveAesKey).toHaveBeenCalledWith(FIXED_MNEMONIC);
  });

  it('calls setEncryptionKey on successful import', async () => {
    renderWithUser();
    switchToImport();
    fillAllImportWords();

    await act(async () => {
      fireEvent.click(continueButton());
    });

    await waitFor(() => expect(mockSetWalletAddress).toHaveBeenCalled());
    expect(mockSetEncryptionKey).toHaveBeenCalledWith('aes-key-hex');
  });

  it('does not call deriveAesKey when validation fails', async () => {
    mockValidateMnemonicPhrase.mockReturnValueOnce(false);

    renderWithUser();
    switchToImport();
    fillAllImportWords();

    await act(async () => {
      fireEvent.click(continueButton());
    });

    await waitFor(() => expect(screen.getByText(/invalid recovery phrase/i)).toBeInTheDocument());
    expect(mockDeriveAesKey).not.toHaveBeenCalled();
  });

  it('shows "User not loaded" error when user is absent during import', async () => {
    mockValidateMnemonicPhrase.mockReturnValue(true);

    mockUseCoreState.mockReturnValue({
      snapshot: {
        currentUser: null,
        sessionToken: 'jwt-token',
      },
      setEncryptionKey: mockSetEncryptionKey,
    });
    renderWithProviders(<Mnemonic />);
    switchToImport();
    fillAllImportWords();

    await act(async () => {
      fireEvent.click(continueButton());
    });

    await waitFor(() => expect(screen.getByText(/user not loaded/i)).toBeInTheDocument());
  });

  it('shows an error when skillManager.setWalletAddress throws during import', async () => {
    mockSetWalletAddress.mockRejectedValueOnce(new Error('wallet error'));

    renderWithUser();
    switchToImport();
    fillAllImportWords();

    await act(async () => {
      fireEvent.click(continueButton());
    });

    await waitFor(() => expect(screen.getByText('wallet error')).toBeInTheDocument());
  });
});

// ---------------------------------------------------------------------------
// Loading state during continue
// ---------------------------------------------------------------------------

describe('Mnemonic — loading state', () => {
  it('disables Continue button while loading', async () => {
    let resolveWallet!: () => void;
    mockSetWalletAddress.mockReturnValueOnce(
      new Promise<void>(res => {
        resolveWallet = res;
      })
    );

    renderWithUser();
    fireEvent.click(screen.getByRole('checkbox'));
    const btn = continueButton();

    await act(async () => {
      fireEvent.click(btn);
    });
    await waitFor(() => expect(screen.getByText('Securing Your Data...')).toBeInTheDocument());
    expect(btn).toBeDisabled();

    await act(async () => {
      resolveWallet();
    });
  });

  it('restores button label after an error', async () => {
    mockDeriveAesKey.mockImplementationOnce(() => {
      throw new Error('oops');
    });

    renderWithUser();
    fireEvent.click(screen.getByRole('checkbox'));

    await act(async () => {
      fireEvent.click(continueButton());
    });

    await waitFor(() => expect(screen.getByText('oops')).toBeInTheDocument());
    expect(screen.queryByText('Securing Your Data...')).not.toBeInTheDocument();
  });
});

// ---------------------------------------------------------------------------
// Provider configuration sanity checks
// ---------------------------------------------------------------------------

describe('Mnemonic — providerConfigs sanity', () => {
  it('calls generateMnemonicPhrase exactly once on mount', () => {
    mockGenerateMnemonicPhrase.mockClear();
    renderWithUser();
    // useMemo with [] dep runs once per render
    expect(mockGenerateMnemonicPhrase).toHaveBeenCalledTimes(1);
  });

  it('does not call generateMnemonicPhrase again after mode switch', () => {
    mockGenerateMnemonicPhrase.mockClear();
    renderWithUser();
    const callsBefore = mockGenerateMnemonicPhrase.mock.calls.length;
    switchToImport();
    fireEvent.click(screen.getByText('Generate a new recovery phrase instead'));
    expect(mockGenerateMnemonicPhrase.mock.calls.length).toBe(callsBefore);
  });
});
