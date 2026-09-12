import { fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { WalletStatus } from '../../../../services/walletApi';
import { renderWithProviders } from '../../../../test/test-utils';
import RecoveryPhrasePanel from '../RecoveryPhrasePanel';

/**
 * Key-custody paths in `RecoveryPhrasePanel` that the existing suite does not
 * reach. `RecoveryPhrasePanel.test.tsx` (34 tests) covers the four modes and the
 * replace-confirm gate thoroughly; measured, it still leaves the panel at
 * 76.6% lines / 73.8% branches, and the uncovered remainder is the part a user
 * depends on to not lose their wallet:
 *
 *   - `handleCopy`'s `document.execCommand` fallback (panel :193-207) — the only
 *     way a seed phrase reaches the clipboard when `navigator.clipboard` is
 *     unavailable (a non-secure context, or a denied permission).
 *   - `handleViewCopy` (:318-332) — the whole function, both paths.
 *   - `handleImportWordChange`'s paste handling (:209-235) — pasting a full
 *     phrase into one slot, which is how most people import.
 *   - the Save gate in generate mode (`canSave`, :357, :442).
 *
 * Two uncovered branches are deliberately NOT tested here because they are
 * unreachable through the UI, and a test that appeared to reach them would be
 * asserting nothing:
 *
 *   - the word-count error at :256-259 (`must be 12, 15, … words (you have N)`).
 *     `isImportComplete` (:354-356) already requires every slot filled AND the
 *     slot count to be a BIP39 length, and it gates the Save button, so
 *     `validateImportPhrase` can never see a bad count.
 *   - `handleSave`'s `!confirmed` early return (:288-291), for the same reason:
 *     `canSave` disables the button first.
 *
 * Both are defensive and harmless; they are recorded in the findings file rather
 * than covered.
 *
 * This is a separate file rather than an edit to the existing one so the two can
 * be reviewed independently.
 */

const PHRASE_12 = 'word1 word2 word3 word4 word5 word6 word7 word8 word9 word10 word11 word12';

const {
  mockGenerateMnemonicPhrase,
  mockFetchWalletStatus,
  mockPersistLocalWalletFromMnemonic,
  mockRevealRecoveryPhrase,
} = vi.hoisted(() => ({
  mockGenerateMnemonicPhrase: vi.fn(() => ''),
  mockFetchWalletStatus: vi.fn(async () => ({}) as WalletStatus),
  mockPersistLocalWalletFromMnemonic: vi.fn(async (_args: unknown) => undefined),
  mockRevealRecoveryPhrase: vi.fn(async () => ({ phrase: '', wordCount: 12 })),
}));

vi.mock('../../../../utils/cryptoKeys', async importOriginal => {
  const original = await importOriginal<typeof import('../../../../utils/cryptoKeys')>();
  return { ...original, generateMnemonicPhrase: mockGenerateMnemonicPhrase };
});

vi.mock('../../../../providers/CoreStateProvider', () => ({
  useCoreState: () => ({
    snapshot: { currentUser: { _id: 'test-user-id' } },
    setEncryptionKey: vi.fn(async () => undefined),
  }),
}));

vi.mock('../../../../services/walletApi', () => ({
  fetchWalletStatus: mockFetchWalletStatus,
  setupLocalWallet: vi.fn(async () => ({}) as WalletStatus),
  revealRecoveryPhrase: mockRevealRecoveryPhrase,
}));

vi.mock('../../../../features/wallet/setupLocalWalletFromMnemonic', () => ({
  persistLocalWalletFromMnemonic: mockPersistLocalWalletFromMnemonic,
}));

const noWallet = (): WalletStatus => ({
  configured: false,
  onboardingCompleted: false,
  consentGranted: false,
  secretStored: false,
  source: null,
  mnemonicWordCount: null,
  accounts: [],
  updatedAtMs: null,
});

const configuredWallet = (): WalletStatus => ({
  configured: true,
  onboardingCompleted: true,
  consentGranted: true,
  secretStored: true,
  source: 'generated',
  mnemonicWordCount: 12,
  accounts: [{ chain: 'evm', address: '0xabc123', derivationPath: "m/44'/60'/0'/0/0" }],
  updatedAtMs: 1_700_000_000_000,
});

/** Install a clipboard whose `writeText` rejects, forcing the legacy fallback. */
function installFailingClipboard() {
  const writeText = vi.fn(async () => {
    throw new Error('clipboard unavailable');
  });
  Object.assign(navigator, { clipboard: { writeText } });
  return writeText;
}

function installWorkingClipboard() {
  const writeText = vi.fn(async () => undefined);
  Object.assign(navigator, { clipboard: { writeText } });
  return writeText;
}

/**
 * jsdom implements neither `execCommand` nor selection on a detached textarea,
 * so the legacy copy path needs both stubbed. `execCommand` returns whatever
 * `ok` says, which is the branch the panel keys `setCopied` off.
 */
function installExecCommand(ok: boolean) {
  // Capture WHAT the legacy path put on the clipboard, not just that it tried.
  // Returning a bare `ok` made both fallback tests pass even if the textarea
  // were empty or held the wrong phrase: the rejected `writeText` proves the
  // fallback was taken, never that it copied the right thing — and a false
  // "Copied" here leaves the user without their recovery phrase. Caught in
  // review by `chatgpt-codex-connector`.
  //
  // The panel appends a hidden textarea, assigns the phrase and selects it
  // before calling copy (`RecoveryPhrasePanel.tsx:198-204`), so at this moment
  // the value is readable from the selected element.
  const copiedValues: string[] = [];
  const execCommand = Object.assign(
    vi.fn((command?: string) => {
      if (command === 'copy') {
        const active = document.activeElement;
        const areas = Array.from(document.body.querySelectorAll('textarea'));
        const source =
          active instanceof HTMLTextAreaElement ? active : (areas[areas.length - 1] ?? null);
        copiedValues.push(source ? source.value : '');
      }
      return ok;
    }),
    { copiedValues }
  );
  Object.defineProperty(document, 'execCommand', {
    value: execCommand,
    configurable: true,
    writable: true,
  });
  return execCommand;
}

/** Drive generate mode to the point where the phrase is revealed. */
async function revealGenerateModePhrase() {
  renderWithProviders(<RecoveryPhrasePanel />);
  await waitFor(() => screen.getByLabelText(/Reveal recovery phrase/i));
  fireEvent.click(screen.getByLabelText(/Reveal recovery phrase/i));
}

const generateCopyButton = () => screen.getByText(/Copy to Clipboard/i).closest('button')!;

beforeEach(() => {
  vi.clearAllMocks();
  mockGenerateMnemonicPhrase.mockReturnValue(PHRASE_12);
  mockFetchWalletStatus.mockResolvedValue(noWallet());
  mockPersistLocalWalletFromMnemonic.mockResolvedValue(undefined);
  mockRevealRecoveryPhrase.mockResolvedValue({ phrase: PHRASE_12, wordCount: 12 });
  installWorkingClipboard();
  installExecCommand(true);
});

afterEach(() => {
  vi.useRealTimers();
});

describe('RecoveryPhrasePanel — clipboard fallback when the async API fails', () => {
  it('falls back to execCommand and still reports Copied', async () => {
    const writeText = installFailingClipboard();
    const execCommand = installExecCommand(true);

    await revealGenerateModePhrase();
    fireEvent.click(generateCopyButton());

    await waitFor(() => expect(screen.getByText(/Copied/i)).toBeInTheDocument());
    expect(writeText).toHaveBeenCalledWith(PHRASE_12);
    expect(execCommand).toHaveBeenCalledWith('copy');
    // The phrase actually reached the clipboard surface — "Copied" is only
    // truthful if the copied text is the phrase itself.
    expect(execCommand.copiedValues).toEqual([PHRASE_12]);
  });

  it('does NOT report Copied when the fallback itself fails', async () => {
    // A user told "Copied" whose clipboard is empty may destroy the only copy of
    // their phrase, so the false branch of `execCommand` has to be honoured.
    installFailingClipboard();
    const execCommand = installExecCommand(false);

    await revealGenerateModePhrase();
    fireEvent.click(generateCopyButton());

    await waitFor(() => expect(execCommand).toHaveBeenCalledWith('copy'));
    expect(screen.queryByText(/^Copied$/i)).not.toBeInTheDocument();
    // The attempt still carried the right phrase — this test is about the
    // panel honouring a FAILED copy, not about it copying the wrong thing.
    expect(execCommand.copiedValues).toEqual([PHRASE_12]);
  });

  it('leaves no textarea behind in the DOM after the fallback runs', async () => {
    installFailingClipboard();
    installExecCommand(true);

    await revealGenerateModePhrase();
    fireEvent.click(generateCopyButton());

    await waitFor(() => expect(screen.getByText(/Copied/i)).toBeInTheDocument());
    // The phrase must not be left sitting in a stray node.
    expect(document.querySelectorAll('textarea')).toHaveLength(0);
  });

  it('does not touch the clipboard at all before the phrase is revealed', async () => {
    const writeText = installWorkingClipboard();
    renderWithProviders(<RecoveryPhrasePanel />);
    await waitFor(() => screen.getByLabelText(/Reveal recovery phrase/i));

    expect(generateCopyButton()).toBeDisabled();
    expect(writeText).not.toHaveBeenCalled();
  });
});

describe('RecoveryPhrasePanel — view-mode copy', () => {
  beforeEach(() => {
    mockFetchWalletStatus.mockResolvedValue(configuredWallet());
  });

  /**
   * view mode → fetch the stored phrase. `handleRevealExistingPhrase` sets both
   * `viewMnemonic` and `viewRevealed` (panel :343-344), so the phrase is already
   * un-blurred here and there is no overlay button to click.
   */
  async function revealViewModePhrase() {
    renderWithProviders(<RecoveryPhrasePanel />);
    await waitFor(() => screen.getByText(/Reveal recovery phrase/i));
    fireEvent.click(screen.getByText(/Reveal recovery phrase/i));
    await waitFor(() => expect(mockRevealRecoveryPhrase).toHaveBeenCalled());
    await waitFor(() => expect(screen.getByText('word1')).toBeInTheDocument());
  }

  it('copies the stored phrase via the async clipboard', async () => {
    const writeText = installWorkingClipboard();
    await revealViewModePhrase();

    const copy = screen.getByText(/Copy/i).closest('button')!;
    await waitFor(() => expect(copy).not.toBeDisabled());
    fireEvent.click(copy);

    await waitFor(() => expect(writeText).toHaveBeenCalledWith(PHRASE_12));
  });

  it('falls back to execCommand in view mode too', async () => {
    installFailingClipboard();
    const execCommand = installExecCommand(true);
    await revealViewModePhrase();

    const copy = screen.getByText(/Copy/i).closest('button')!;
    await waitFor(() => expect(copy).not.toBeDisabled());
    fireEvent.click(copy);

    await waitFor(() => expect(execCommand).toHaveBeenCalledWith('copy'));
  });

  it('offers no copy affordance at all until the phrase has been fetched', async () => {
    const writeText = installWorkingClipboard();
    renderWithProviders(<RecoveryPhrasePanel />);
    await waitFor(() => screen.getByText(/Reveal recovery phrase/i));

    // The copy button lives inside the `viewMnemonic` block (ViewMode :109),
    // so with no phrase in state there is nothing to click.
    expect(screen.queryByText(/Copy/i)).not.toBeInTheDocument();
    expect(writeText).not.toHaveBeenCalled();
    expect(mockRevealRecoveryPhrase).not.toHaveBeenCalled();
  });
});

describe('RecoveryPhrasePanel — pasting a whole phrase into the import grid', () => {
  async function enterImportMode() {
    renderWithProviders(<RecoveryPhrasePanel />);
    await waitFor(() => screen.getByText(/I already have a recovery phrase/i));
    fireEvent.click(screen.getByText(/I already have a recovery phrase/i));
    await waitFor(() => screen.getByText(/Enter your recovery phrase below/i));
    return screen.getAllByLabelText(/Recovery phrase word/i) as HTMLInputElement[];
  }

  it('spreads a pasted 12-word phrase across all twelve slots', async () => {
    const inputs = await enterImportMode();
    fireEvent.change(inputs[0], { target: { value: PHRASE_12 } });

    const after = screen.getAllByLabelText(/Recovery phrase word/i) as HTMLInputElement[];
    expect(after.map(i => i.value)).toEqual(PHRASE_12.split(' '));
  });

  it('lowercases a pasted phrase', async () => {
    const inputs = await enterImportMode();
    fireEvent.change(inputs[0], { target: { value: PHRASE_12.toUpperCase() } });

    const after = screen.getAllByLabelText(/Recovery phrase word/i) as HTMLInputElement[];
    expect(after[0].value).toBe('word1');
    expect(after[11].value).toBe('word12');
  });

  it('tolerates newlines and repeated spaces in a pasted phrase', async () => {
    const inputs = await enterImportMode();
    fireEvent.change(inputs[0], { target: { value: `  ${PHRASE_12.replace(/ /g, '\n  ')}  ` } });

    const after = screen.getAllByLabelText(/Recovery phrase word/i) as HTMLInputElement[];
    expect(after.map(i => i.value)).toEqual(PHRASE_12.split(' '));
  });

  it('grows the grid when a 24-word phrase is pasted into a 12-slot grid', async () => {
    const inputs = await enterImportMode();
    const words24 = Array.from({ length: 24 }, (_, i) => `w${i + 1}`);
    fireEvent.change(inputs[0], { target: { value: words24.join(' ') } });

    const after = screen.getAllByLabelText(/Recovery phrase word/i) as HTMLInputElement[];
    expect(after).toHaveLength(24);
    expect(after.map(i => i.value)).toEqual(words24);
  });

  it('fills forward from the pasted slot when the word count is not a BIP39 length', async () => {
    // Three words is not a valid phrase length, so this is a partial paste and
    // must not clobber the whole grid.
    const inputs = await enterImportMode();
    fireEvent.change(inputs[2], { target: { value: 'alpha beta gamma' } });

    const after = screen.getAllByLabelText(/Recovery phrase word/i) as HTMLInputElement[];
    expect(after).toHaveLength(12);
    expect(after[0].value).toBe('');
    expect(after[1].value).toBe('');
    expect(after[2].value).toBe('alpha');
    expect(after[3].value).toBe('beta');
    expect(after[4].value).toBe('gamma');
    expect(after[5].value).toBe('');
  });

  it('does not write past the end of the grid on an over-long partial paste', async () => {
    const inputs = await enterImportMode();
    // Ten words pasted into slot 10 of 12: only two slots remain.
    const words = Array.from({ length: 10 }, (_, i) => `p${i + 1}`);
    fireEvent.change(inputs[10], { target: { value: words.join(' ') } });

    const after = screen.getAllByLabelText(/Recovery phrase word/i) as HTMLInputElement[];
    expect(after).toHaveLength(12);
    expect(after[10].value).toBe('p1');
    expect(after[11].value).toBe('p2');
  });
});

describe('RecoveryPhrasePanel — generate mode will not save without the confirmation', () => {
  // NOTE: the enforcement is `canSave` gating the button's `disabled`
  // (panel :357, :442), NOT the `!confirmed` early return inside `handleSave`
  // — that return is unreachable while the button is disabled. Assert the
  // reachable guard rather than pretending to exercise the other one.
  it('leaves Save disabled while the confirm checkbox is unticked', async () => {
    renderWithProviders(<RecoveryPhrasePanel />);
    await waitFor(() => screen.getByRole('checkbox'));
    expect(screen.getByRole('checkbox')).not.toBeChecked();

    expect(screen.getByText(/^Save/i).closest('button')!).toBeDisabled();
    expect(mockPersistLocalWalletFromMnemonic).not.toHaveBeenCalled();
  });

  it('enables Save the moment the confirmation is ticked', async () => {
    renderWithProviders(<RecoveryPhrasePanel />);
    await waitFor(() => screen.getByRole('checkbox'));
    fireEvent.click(screen.getByRole('checkbox'));
    expect(screen.getByText(/^Save/i).closest('button')!).not.toBeDisabled();
  });

  it('persists once the confirmation is ticked', async () => {
    renderWithProviders(<RecoveryPhrasePanel />);
    await waitFor(() => screen.getByRole('checkbox'));
    fireEvent.click(screen.getByRole('checkbox'));
    fireEvent.click(screen.getByText(/^Save/i).closest('button')!);

    await waitFor(() => expect(mockPersistLocalWalletFromMnemonic).toHaveBeenCalledTimes(1));
    expect(mockPersistLocalWalletFromMnemonic).toHaveBeenCalledWith(
      expect.objectContaining({ mnemonic: PHRASE_12, source: 'generated' })
    );
  });
});

describe('RecoveryPhrasePanel — the Copied indicator resets itself', () => {
  it('clears the Copied state after three seconds', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    installWorkingClipboard();

    renderWithProviders(<RecoveryPhrasePanel />);
    await waitFor(() => screen.getByLabelText(/Reveal recovery phrase/i));
    fireEvent.click(screen.getByLabelText(/Reveal recovery phrase/i));
    fireEvent.click(generateCopyButton());

    await waitFor(() => expect(screen.getByText(/Copied/i)).toBeInTheDocument());
    await vi.advanceTimersByTimeAsync(3100);
    await waitFor(() => expect(screen.queryByText(/^Copied$/i)).not.toBeInTheDocument());
  });
});
