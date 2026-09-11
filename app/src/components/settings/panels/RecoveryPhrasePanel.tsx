import { type KeyboardEvent, useCallback, useEffect, useRef, useState } from 'react';

import { persistLocalWalletFromMnemonic } from '../../../features/wallet/setupLocalWalletFromMnemonic';
import { useT } from '../../../lib/i18n/I18nContext';
import { useCoreState } from '../../../providers/CoreStateProvider';
import {
  fetchWalletStatus,
  revealRecoveryPhrase,
  type WalletStatus,
} from '../../../services/walletApi';
import {
  generateMnemonicPhrase,
  MNEMONIC_GENERATE_WORD_COUNT,
  validateMnemonicPhrase,
} from '../../../utils/cryptoKeys';
import { Alert } from '../../ui/Alert';
import Button from '../../ui/Button';
import { CheckIcon, Spinner } from '../../ui/icons';
import { CenteredLoadingState } from '../../ui/LoadingState';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import SettingsPanel from '../layout/SettingsPanel';
import RecoveryPhraseGenerateMode from './RecoveryPhraseGenerateMode';
import RecoveryPhraseImportMode from './RecoveryPhraseImportMode';
import RecoveryPhraseReplaceConfirm from './RecoveryPhraseReplaceConfirm';
import RecoveryPhraseViewMode from './RecoveryPhraseViewMode';

const BIP39_IMPORT_LENGTHS = [12, 15, 18, 21, 24] as const;

const IMPORT_SLOTS_INITIAL = MNEMONIC_GENERATE_WORD_COUNT;

// Panel mode flow:
// - 'loading': initial — fetching wallet status.
// - 'view': existing wallet found — shows metadata, no mnemonic displayed.
// - 'replace-confirm': user clicked "Replace wallet" — shows warning dialog.
// - 'generate': no wallet (or post-confirm replace) — generate new phrase flow.
// - 'import': import an existing phrase.
type PanelMode = 'loading' | 'view' | 'replace-confirm' | 'generate' | 'import';

const RecoveryPhrasePanel = () => {
  const { t } = useT();
  const { navigateBack } = useSettingsNavigation();
  const { snapshot, setEncryptionKey } = useCoreState();
  const user = snapshot.currentUser;

  const [mode, setMode] = useState<PanelMode>('loading');
  const [walletStatus, setWalletStatus] = useState<WalletStatus | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);

  // Generate mode state
  const [mnemonic, setMnemonic] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [confirmed, setConfirmed] = useState(false);
  const [revealed, setRevealed] = useState(false);

  // Replace-mode state: tracks that the user went through the replace flow
  const [isReplace, setIsReplace] = useState(false);

  // View mode: reveal existing phrase
  const [viewRevealed, setViewRevealed] = useState(false);
  const [viewMnemonic, setViewMnemonic] = useState<string | null>(null);
  const [viewRevealLoading, setViewRevealLoading] = useState(false);
  const [viewRevealError, setViewRevealError] = useState<string | null>(null);
  const [viewCopied, setViewCopied] = useState(false);

  // Import mode state
  const [selectedWordCount, setSelectedWordCount] = useState(IMPORT_SLOTS_INITIAL);
  const [importWords, setImportWords] = useState<string[]>(Array(IMPORT_SLOTS_INITIAL).fill(''));
  const [importValid, setImportValid] = useState<boolean | null>(null);

  // Shared
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState(false);

  const inputRefs = useRef<(HTMLInputElement | null)[]>([]);

  // ── On mount: check for existing wallet ──────────────────────────────────
  useEffect(() => {
    let cancelled = false;
    const checkWallet = async () => {
      try {
        const status = await fetchWalletStatus();
        if (cancelled) return;
        setWalletStatus(status);
        if (status.configured && status.onboardingCompleted) {
          setMode('view');
        } else {
          // No configured wallet — generate mode. Generate phrase now.
          const phrase = generateMnemonicPhrase();
          setMnemonic(phrase);
          setMode('generate');
        }
      } catch (e) {
        if (cancelled) return;
        // If status fetch fails, degrade gracefully: show error in view mode.
        // Do NOT silently generate a phrase that could overwrite an existing wallet.
        setStatusError(
          e instanceof Error ? e.message : 'Failed to check wallet status. Please try again.'
        );
        setMode('view');
      }
    };
    void checkWallet();
    return () => {
      cancelled = true;
    };
  }, []);

  // ── Transition into generate mode after replace confirmation ─────────────
  const handleConfirmReplace = useCallback(() => {
    const phrase = generateMnemonicPhrase();
    setMnemonic(phrase);
    setIsReplace(true);
    setConfirmed(false);
    setRevealed(false);
    setError(null);
    setMode('generate');
  }, []);

  // ── Transition into import mode after replace confirmation ────────────────
  const handleImportReplace = useCallback(() => {
    setIsReplace(true);
    setImportValid(null);
    setError(null);
    setSelectedWordCount(IMPORT_SLOTS_INITIAL);
    setImportWords(Array(IMPORT_SLOTS_INITIAL).fill(''));
    setMode('import');
  }, []);

  useEffect(() => {
    if (copied) {
      const timer = setTimeout(() => setCopied(false), 3000);
      return () => clearTimeout(timer);
    }
  }, [copied]);

  useEffect(() => {
    if (viewCopied) {
      const timer = setTimeout(() => setViewCopied(false), 3000);
      return () => clearTimeout(timer);
    }
  }, [viewCopied]);

  // Security: clear plaintext phrase from state when unmounting.
  useEffect(() => {
    return () => {
      setViewMnemonic(null);
      setViewRevealed(false);
    };
  }, []);

  // Clear phrase when navigating away from view mode.
  useEffect(() => {
    if (mode !== 'view') {
      setViewMnemonic(null);
      setViewRevealed(false);
      setViewRevealError(null);
    }
  }, [mode]);

  const switchMode = useCallback((nextMode: 'generate' | 'import') => {
    setMode(nextMode);
    setConfirmed(false);
    setError(null);
    setImportValid(null);
    setSelectedWordCount(IMPORT_SLOTS_INITIAL);
    setImportWords(Array(IMPORT_SLOTS_INITIAL).fill(''));
  }, []);

  const handleWordCountChange = useCallback((count: number) => {
    setSelectedWordCount(count);
    setImportWords(prev => {
      const newWords = Array(count).fill('');
      for (let i = 0; i < Math.min(prev.length, count); i++) {
        newWords[i] = prev[i];
      }
      return newWords;
    });
    setImportValid(null);
    setError(null);
  }, []);

  useEffect(() => {
    if (success) {
      const timer = setTimeout(() => {
        navigateBack();
      }, 1500);
      return () => clearTimeout(timer);
    }
  }, [success, navigateBack]);

  const handleCopy = useCallback(async () => {
    if (!mnemonic) return;
    try {
      await navigator.clipboard.writeText(mnemonic);
      setCopied(true);
    } catch {
      const textarea = document.createElement('textarea');
      textarea.value = mnemonic;
      textarea.style.position = 'fixed';
      textarea.style.opacity = '0';
      document.body.appendChild(textarea);
      textarea.select();
      const ok = document.execCommand('copy');
      document.body.removeChild(textarea);
      if (ok) setCopied(true);
    }
  }, [mnemonic]);

  const handleImportWordChange = useCallback(
    (index: number, value: string) => {
      const pastedWords = value.trim().split(/\s+/).filter(Boolean);
      if (pastedWords.length > 1) {
        const fullPhraseLen = pastedWords.length;
        if (BIP39_IMPORT_LENGTHS.includes(fullPhraseLen as (typeof BIP39_IMPORT_LENGTHS)[number])) {
          setImportWords(pastedWords.map(w => w.toLowerCase()));
          setImportValid(null);
          inputRefs.current[fullPhraseLen - 1]?.focus();
          return;
        }
        const newWords = [...importWords];
        const slotCount = newWords.length;
        for (let i = 0; i < Math.min(pastedWords.length, slotCount - index); i++) {
          newWords[index + i] = pastedWords[i].toLowerCase();
        }
        setImportWords(newWords);
        setImportValid(null);
        const nextEmpty = newWords.findIndex(w => !w);
        const focusIndex = nextEmpty === -1 ? slotCount - 1 : nextEmpty;
        inputRefs.current[focusIndex]?.focus();
        return;
      }

      const newWords = [...importWords];
      newWords[index] = value.toLowerCase().trim();
      setImportWords(newWords);
      setImportValid(null);
    },
    [importWords]
  );

  const handleImportKeyDown = useCallback(
    (index: number, e: KeyboardEvent<HTMLInputElement>) => {
      if (e.key === 'Backspace' && !importWords[index] && index > 0) {
        inputRefs.current[index - 1]?.focus();
      }
    },
    [importWords]
  );

  const handleValidateImport = useCallback(() => {
    const phrase = importWords.join(' ').trim();
    const filledWords = importWords.filter(w => w.trim());
    const n = filledWords.length;

    if (!BIP39_IMPORT_LENGTHS.includes(n as (typeof BIP39_IMPORT_LENGTHS)[number])) {
      setError(`Recovery phrase must be ${BIP39_IMPORT_LENGTHS.join(', ')} words (you have ${n}).`);
      setImportValid(false);
      return false;
    }

    const isValid = validateMnemonicPhrase(phrase);
    setImportValid(isValid);

    if (!isValid) {
      setError(t('mnemonic.invalidPhrase'));
      return false;
    }

    setError(null);
    return true;
  }, [importWords, t]);

  const handleSave = async () => {
    setError(null);
    setLoading(true);

    try {
      let phraseToUse: string;

      if (mode === 'import') {
        if (!handleValidateImport()) {
          setLoading(false);
          return;
        }
        phraseToUse = importWords.join(' ').trim();
      } else {
        if (!confirmed) {
          setLoading(false);
          return;
        }
        if (!mnemonic) {
          setLoading(false);
          return;
        }
        phraseToUse = mnemonic;
      }

      if (!user?._id) {
        setError(t('mnemonic.userNotLoaded'));
        return;
      }
      await persistLocalWalletFromMnemonic({
        mnemonic: phraseToUse,
        source: mode === 'generate' ? 'generated' : 'imported',
        setEncryptionKey,
        // Only pass force=true when the user has gone through the replace confirmation flow.
        force: isReplace ? true : undefined,
      });
      setSuccess(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : t('mnemonic.somethingWentWrong'));
    } finally {
      setLoading(false);
    }
  };

  const handleViewCopy = useCallback(async () => {
    if (!viewMnemonic) return;
    try {
      await navigator.clipboard.writeText(viewMnemonic);
      setViewCopied(true);
    } catch {
      const textarea = document.createElement('textarea');
      textarea.value = viewMnemonic;
      textarea.style.position = 'fixed';
      textarea.style.opacity = '0';
      document.body.appendChild(textarea);
      textarea.select();
      const ok = document.execCommand('copy');
      document.body.removeChild(textarea);
      if (ok) setViewCopied(true);
    }
  }, [viewMnemonic]);

  const handleRevealExistingPhrase = useCallback(async () => {
    setViewRevealLoading(true);
    setViewRevealError(null);
    setViewMnemonic(null);
    setViewRevealed(false);
    try {
      const result = await revealRecoveryPhrase();
      setViewMnemonic(result.phrase);
      setViewRevealed(true);
    } catch (e) {
      setViewRevealError(e instanceof Error ? e.message : t('mnemonic.somethingWentWrong'));
    } finally {
      setViewRevealLoading(false);
    }
  }, [t]);

  const words = mnemonic ? mnemonic.split(' ') : [];
  const importWordCount = importWords.filter(w => w.trim()).length;
  const isImportComplete =
    importWords.every(w => w.trim()) &&
    BIP39_IMPORT_LENGTHS.includes(importWordCount as (typeof BIP39_IMPORT_LENGTHS)[number]);
  const canSave = mode === 'generate' ? confirmed : isImportComplete;

  return (
    <SettingsPanel
      description={t('pages.settings.account.recoveryPhraseDesc')}
      testId="recovery-phrase-panel">
      {success ? (
        <div className="flex flex-col items-center justify-center gap-3 py-12">
          <div className="w-12 h-12 rounded-full bg-sage-500/20 flex items-center justify-center">
            <CheckIcon className="w-6 h-6 text-sage-400" />
          </div>
          <p className="text-sm font-medium text-sage-500">{t('mnemonic.phraseSaved')}</p>
          <p className="text-xs text-content-muted">{t('mnemonic.walletReady')}</p>
        </div>
      ) : (
        <>
          {mode === 'loading' && (
            <CenteredLoadingState label={t('mnemonic.loadingWalletStatus')} className="py-12" />
          )}

          {mode === 'view' && (
            <RecoveryPhraseViewMode
              statusError={statusError}
              walletStatus={walletStatus}
              viewMnemonic={viewMnemonic}
              viewRevealed={viewRevealed}
              onRevealBlur={() => setViewRevealed(true)}
              onHide={() => {
                setViewMnemonic(null);
                setViewRevealed(false);
              }}
              viewRevealLoading={viewRevealLoading}
              viewRevealError={viewRevealError}
              onReveal={() => void handleRevealExistingPhrase()}
              viewCopied={viewCopied}
              onCopy={() => void handleViewCopy()}
              onReplaceClick={() => setMode('replace-confirm')}
            />
          )}

          {mode === 'replace-confirm' && (
            <RecoveryPhraseReplaceConfirm
              onConfirmReplace={handleConfirmReplace}
              onImportInstead={handleImportReplace}
              onCancel={() => setMode('view')}
            />
          )}

          {(mode === 'generate' || mode === 'import') && (
            <>
              {mode === 'generate' ? (
                <RecoveryPhraseGenerateMode
                  words={words}
                  revealed={revealed}
                  onReveal={() => setRevealed(true)}
                  copied={copied}
                  onCopy={() => void handleCopy()}
                  confirmed={confirmed}
                  onConfirmedChange={setConfirmed}
                  onSwitchToImport={() => switchMode('import')}
                />
              ) : (
                <RecoveryPhraseImportMode
                  importWords={importWords}
                  selectedWordCount={selectedWordCount}
                  importValid={importValid}
                  inputRefs={inputRefs}
                  onWordCountChange={handleWordCountChange}
                  onWordChange={handleImportWordChange}
                  onWordKeyDown={handleImportKeyDown}
                  onSwitchToGenerate={() => switchMode('generate')}
                />
              )}

              {error && (
                <Alert variant="destructive" className="mb-3">
                  <p className="text-xs leading-relaxed">{error}</p>
                </Alert>
              )}

              <Button
                type="button"
                variant="primary"
                size="lg"
                onClick={() => void handleSave()}
                disabled={!canSave || loading}
                className="w-full">
                {loading ? (
                  <>
                    <Spinner className="w-4 h-4" />
                    <span>{t('mnemonic.securingData')}</span>
                  </>
                ) : (
                  t('mnemonic.saveRecoveryPhrase')
                )}
              </Button>
            </>
          )}
        </>
      )}
    </SettingsPanel>
  );
};

export default RecoveryPhrasePanel;
