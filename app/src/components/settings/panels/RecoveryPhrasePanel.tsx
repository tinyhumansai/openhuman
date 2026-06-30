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
import Button from '../../ui/Button';
import { SettingsCheckbox } from '../controls';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import SettingsPanel from '../layout/SettingsPanel';

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

  // ── Render helpers ────────────────────────────────────────────────────────

  const renderLoading = () => (
    <div className="flex flex-col items-center justify-center gap-3 py-12">
      <svg className="w-6 h-6 animate-spin text-primary-400" fill="none" viewBox="0 0 24 24">
        <circle
          className="opacity-25"
          cx="12"
          cy="12"
          r="10"
          stroke="currentColor"
          strokeWidth="4"
        />
        <path
          className="opacity-75"
          fill="currentColor"
          d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
        />
      </svg>
      <p className="text-sm text-content-muted">{t('mnemonic.loadingWalletStatus')}</p>
    </div>
  );

  const renderViewMode = () => (
    <div className="space-y-5">
      {statusError ? (
        <div
          role="alert"
          className="flex items-start gap-2.5 p-3 rounded-xl bg-coral-50 dark:bg-coral-500/10 border border-coral-200 dark:border-coral-500/30">
          <svg
            className="w-4 h-4 text-coral-500 flex-shrink-0 mt-0.5"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            strokeWidth={2}>
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"
            />
          </svg>
          <p className="text-xs text-coral-700 dark:text-coral-300 leading-relaxed">
            {statusError}
          </p>
        </div>
      ) : (
        <>
          {/* Wallet configured banner */}
          <div className="flex items-start gap-2.5 p-3 rounded-xl bg-sage-50 dark:bg-sage-500/10 border border-sage-200 dark:border-sage-500/30">
            <svg
              className="w-4 h-4 text-sage-500 flex-shrink-0 mt-0.5"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
            </svg>
            <p className="text-xs text-sage-700 dark:text-sage-300 leading-relaxed font-medium">
              {t('mnemonic.walletAlreadyConfigured')}
            </p>
          </div>

          {/* Wallet metadata */}
          {walletStatus && (
            <div className="bg-surface-muted rounded-2xl p-4 border border-line space-y-3">
              {walletStatus.source && (
                <div className="flex items-center justify-between">
                  <span className="text-xs text-content-muted">{t('mnemonic.walletSource')}</span>
                  <span className="text-xs font-medium text-content-secondary capitalize">
                    {walletStatus.source}
                  </span>
                </div>
              )}
              {walletStatus.mnemonicWordCount && (
                <div className="flex items-center justify-between">
                  <span className="text-xs text-content-muted">
                    {t('mnemonic.walletWordCount')}
                  </span>
                  <span className="text-xs font-medium text-content-secondary">
                    {walletStatus.mnemonicWordCount} words
                  </span>
                </div>
              )}
              {walletStatus.updatedAtMs && (
                <div className="flex items-center justify-between">
                  <span className="text-xs text-content-muted">
                    {t('mnemonic.walletLastUpdated')}
                  </span>
                  <span className="text-xs font-medium text-content-secondary">
                    {new Date(walletStatus.updatedAtMs).toLocaleDateString()}
                  </span>
                </div>
              )}
              {walletStatus.accounts.length > 0 && (
                <div>
                  <span className="text-xs text-content-muted block mb-2">
                    {t('mnemonic.viewAccounts')}
                  </span>
                  <div className="space-y-1.5">
                    {walletStatus.accounts.map(account => (
                      <div key={account.chain} className="flex items-center justify-between gap-2">
                        <span className="text-xs font-mono font-medium uppercase text-content-muted w-14 shrink-0">
                          {account.chain}
                        </span>
                        <span className="text-xs font-mono text-content-secondary truncate">
                          {account.address}
                        </span>
                      </div>
                    ))}
                  </div>
                </div>
              )}
            </div>
          )}

          {/* Reveal existing recovery phrase */}
          {viewMnemonic ? (
            <div className="space-y-3">
              <div className="flex items-start gap-2.5 p-3 rounded-xl bg-amber-50 dark:bg-amber-500/10 border border-amber-200 dark:border-amber-500/30">
                <svg
                  className="w-4 h-4 text-amber-600 dark:text-amber-300 flex-shrink-0 mt-0.5"
                  fill="none"
                  viewBox="0 0 24 24"
                  stroke="currentColor"
                  strokeWidth={2}>
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"
                  />
                </svg>
                <p className="text-xs text-amber-800 dark:text-amber-200 leading-relaxed">
                  {t('mnemonic.cannotRecover')}
                </p>
              </div>
              <div className="bg-surface-muted rounded-2xl p-4 border border-line relative">
                <div
                  className="grid grid-cols-3 gap-2 transition-all duration-300"
                  style={{
                    filter: viewRevealed ? 'none' : 'blur(8px)',
                    userSelect: viewRevealed ? 'auto' : 'none',
                    pointerEvents: viewRevealed ? 'auto' : 'none',
                  }}>
                  {viewMnemonic.split(' ').map((word, index) => (
                    <div
                      key={index}
                      className="flex items-center gap-2 bg-surface rounded-lg px-3 py-2 text-sm border border-line">
                      <span className="text-content-muted font-mono text-xs w-5 text-right">
                        {index + 1}.
                      </span>
                      <span className="font-mono font-medium">{word}</span>
                    </div>
                  ))}
                </div>
                {!viewRevealed && (
                  <button
                    type="button"
                    onClick={() => setViewRevealed(true)}
                    aria-label={t('mnemonic.revealPhrase')}
                    className="absolute inset-0 flex items-center justify-center cursor-pointer bg-transparent">
                    <svg
                      className="w-7 h-7 text-content dark:text-white transition-opacity duration-200 hover:opacity-70"
                      fill="none"
                      viewBox="0 0 24 24"
                      stroke="currentColor"
                      strokeWidth={1.5}>
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        d="M17.94 17.94A10.07 10.07 0 0112 20c-7 0-11-8-11-8a18.45 18.45 0 015.06-5.94M9.9 4.24A9.12 9.12 0 0112 4c7 0 11 8 11 8a18.5 18.5 0 01-2.16 3.19m-6.72-1.07a3 3 0 11-4.24-4.24"
                      />
                      <line x1="1" y1="1" x2="23" y2="23" />
                    </svg>
                  </button>
                )}
              </div>
              <Button
                type="button"
                variant="secondary"
                size="md"
                onClick={() => void handleViewCopy()}
                disabled={!viewRevealed}
                className="w-full">
                {viewCopied ? (
                  <>
                    <svg
                      className="w-4 h-4 text-sage-400"
                      fill="none"
                      viewBox="0 0 24 24"
                      stroke="currentColor"
                      strokeWidth={2}>
                      <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
                    </svg>
                    <span className="text-sage-400">{t('common.copied')}</span>
                  </>
                ) : (
                  <>
                    <svg
                      className="w-4 h-4"
                      fill="none"
                      viewBox="0 0 24 24"
                      stroke="currentColor"
                      strokeWidth={2}>
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z"
                      />
                    </svg>
                    <span>{t('mnemonic.copyToClipboard')}</span>
                  </>
                )}
              </Button>
              <Button
                type="button"
                variant="tertiary"
                onClick={() => {
                  setViewMnemonic(null);
                  setViewRevealed(false);
                }}
                className="w-full">
                {t('mnemonic.hidePhrase')}
              </Button>
            </div>
          ) : (
            <>
              {viewRevealError && (
                <div
                  role="alert"
                  className="flex items-start gap-2.5 p-3 rounded-xl bg-coral-50 dark:bg-coral-500/10 border border-coral-200 dark:border-coral-500/30">
                  <svg
                    className="w-4 h-4 text-coral-500 flex-shrink-0 mt-0.5"
                    fill="none"
                    viewBox="0 0 24 24"
                    stroke="currentColor"
                    strokeWidth={2}>
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"
                    />
                  </svg>
                  <p className="text-xs text-coral-700 dark:text-coral-300 leading-relaxed">
                    {viewRevealError}
                  </p>
                </div>
              )}
              <Button
                type="button"
                variant="secondary"
                size="md"
                onClick={() => void handleRevealExistingPhrase()}
                disabled={viewRevealLoading}
                className="w-full">
                {viewRevealLoading ? (
                  <>
                    <svg className="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
                      <circle
                        className="opacity-25"
                        cx="12"
                        cy="12"
                        r="10"
                        stroke="currentColor"
                        strokeWidth="4"
                      />
                      <path
                        className="opacity-75"
                        fill="currentColor"
                        d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
                      />
                    </svg>
                    <span>{t('mnemonic.loadingWalletStatus')}</span>
                  </>
                ) : (
                  t('mnemonic.revealRecoveryPhrase')
                )}
              </Button>
            </>
          )}

          {/* Replace wallet CTA */}
          <Button
            type="button"
            variant="secondary"
            size="md"
            onClick={() => setMode('replace-confirm')}
            className="w-full">
            {t('mnemonic.replaceWallet')}
          </Button>
        </>
      )}
    </div>
  );

  const renderReplaceConfirm = () => (
    <div className="space-y-5">
      {/* Danger warning */}
      <div className="flex items-start gap-2.5 p-4 rounded-xl bg-coral-50 dark:bg-coral-500/10 border border-coral-200 dark:border-coral-500/30">
        <svg
          className="w-5 h-5 text-coral-500 flex-shrink-0 mt-0.5"
          fill="none"
          viewBox="0 0 24 24"
          stroke="currentColor"
          strokeWidth={2}>
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"
          />
        </svg>
        <p className="text-sm text-coral-800 dark:text-coral-200 leading-relaxed">
          {t('mnemonic.replaceWalletWarning')}
        </p>
      </div>

      {/* Replace confirmation button */}
      <Button
        type="button"
        variant="primary"
        size="md"
        onClick={handleConfirmReplace}
        className="w-full !bg-coral-500 hover:!bg-coral-600 border-coral-500">
        {t('mnemonic.replaceWalletConfirm')}
      </Button>

      {/* Import instead */}
      <Button type="button" variant="tertiary" onClick={handleImportReplace} className="w-full">
        {t('mnemonic.alreadyHavePhrase')}
      </Button>

      {/* Cancel */}
      <Button type="button" variant="tertiary" onClick={() => setMode('view')} className="w-full">
        {t('common.cancel')}
      </Button>
    </div>
  );

  const renderGenerateMode = () => (
    <>
      <div className="mb-4 space-y-3">
        <p className="text-sm text-content-secondary leading-relaxed">
          {t('mnemonic.writeDownWords')} {MNEMONIC_GENERATE_WORD_COUNT} {t('mnemonic.wordsInOrder')}
        </p>
        <div className="flex items-start gap-2.5 p-3 rounded-xl bg-amber-50 dark:bg-amber-500/10 border border-amber-200 dark:border-amber-500/30">
          <svg
            className="w-4 h-4 text-amber-600 dark:text-amber-300 flex-shrink-0 mt-0.5"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            strokeWidth={2}>
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"
            />
          </svg>
          <p className="text-xs text-amber-800 dark:text-amber-200 leading-relaxed">
            {t('mnemonic.cannotRecover')}
          </p>
        </div>
      </div>

      <div className="bg-surface-muted rounded-2xl p-4 mb-4 border border-line relative">
        <div
          className="grid grid-cols-3 gap-2 transition-all duration-300"
          style={{
            filter: revealed ? 'none' : 'blur(8px)',
            userSelect: revealed ? 'auto' : 'none',
            pointerEvents: revealed ? 'auto' : 'none',
          }}>
          {words.map((word, index) => (
            <div
              key={index}
              className="flex items-center gap-2 bg-surface rounded-lg px-3 py-2 text-sm border border-line">
              <span className="text-content-muted font-mono text-xs w-5 text-right">
                {index + 1}.
              </span>
              <span className="font-mono font-medium">{word}</span>
            </div>
          ))}
        </div>
        {!revealed && (
          <button
            type="button"
            onClick={() => setRevealed(true)}
            aria-label={t('mnemonic.revealPhrase')}
            className="absolute inset-0 flex items-center justify-center cursor-pointer bg-transparent">
            <svg
              className="w-7 h-7 text-content dark:text-white transition-opacity duration-200 hover:opacity-70"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              strokeWidth={1.5}>
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                d="M17.94 17.94A10.07 10.07 0 0112 20c-7 0-11-8-11-8a18.45 18.45 0 015.06-5.94M9.9 4.24A9.12 9.12 0 0112 4c7 0 11 8 11 8a18.5 18.5 0 01-2.16 3.19m-6.72-1.07a3 3 0 11-4.24-4.24"
              />
              <line x1="1" y1="1" x2="23" y2="23" />
            </svg>
          </button>
        )}
      </div>

      <Button
        type="button"
        variant="secondary"
        size="md"
        onClick={() => void handleCopy()}
        disabled={!revealed}
        className="w-full mb-3">
        {copied ? (
          <>
            <svg
              className="w-4 h-4 text-sage-400"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
            </svg>
            <span className="text-sage-400">{t('common.copied')}</span>
          </>
        ) : (
          <>
            <svg
              className="w-4 h-4"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              strokeWidth={2}>
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z"
              />
            </svg>
            <span>{t('mnemonic.copyToClipboard')}</span>
          </>
        )}
      </Button>

      <Button
        type="button"
        variant="tertiary"
        onClick={() => switchMode('import')}
        className="w-full mb-3">
        {t('mnemonic.alreadyHavePhrase')}
      </Button>

      <label className="flex items-start gap-3 cursor-pointer mb-4">
        <SettingsCheckbox
          id="mnemonic-confirm-checkbox"
          checked={confirmed}
          onCheckedChange={setConfirmed}
        />
        <span className="text-sm text-content-secondary">{t('mnemonic.consentSaved')}</span>
      </label>
    </>
  );

  const renderImportMode = () => (
    <>
      <div className="mb-4">
        <p className="text-sm text-content-secondary leading-relaxed">
          {t('mnemonic.enterPhraseToRestore')}
        </p>
      </div>

      <div className="flex items-center gap-2 mb-3">
        <span className="text-xs text-content-muted">{t('mnemonic.words')}:</span>
        {BIP39_IMPORT_LENGTHS.map(len => (
          <button
            key={len}
            type="button"
            onClick={() => handleWordCountChange(len)}
            className={`px-2.5 py-1 text-xs font-medium rounded-lg transition-colors ${
              selectedWordCount === len
                ? 'bg-primary-500/20 border-primary-500/40 text-primary-600 dark:text-primary-300 border'
                : 'border border-line text-content-muted hover:border-line-strong dark:border-line-strong'
            }`}>
            {len}
          </button>
        ))}
      </div>

      <div className="bg-surface-muted rounded-2xl p-4 mb-4 border border-line">
        <div className="grid grid-cols-3 gap-2">
          {importWords.map((word, index) => (
            <div key={index} className="flex items-center gap-1.5">
              <span className="text-content-muted font-mono text-xs w-5 text-right shrink-0">
                {index + 1}.
              </span>
              <input
                aria-label={`Recovery phrase word ${index + 1}`}
                ref={el => {
                  inputRefs.current[index] = el;
                }}
                type="text"
                value={word}
                onChange={e => handleImportWordChange(index, e.target.value)}
                onKeyDown={e => handleImportKeyDown(index, e)}
                autoComplete="off"
                spellCheck={false}
                className={`w-full font-mono text-sm font-medium px-2 py-1.5 rounded-lg border bg-surface text-content outline-none transition-colors ${
                  importValid === false && word.trim()
                    ? 'border-coral-400 focus:border-coral-300 dark:border-coral-500/40'
                    : importValid === true
                      ? 'border-sage-400 focus:border-sage-300 dark:border-sage-500/40'
                      : 'border-line focus:border-primary-400'
                }`}
              />
            </div>
          ))}
        </div>
      </div>

      {importValid === true && (
        <div className="flex items-center gap-2 text-sage-400 text-sm mb-3 justify-center">
          <svg
            className="w-4 h-4"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
          </svg>
          <span>{t('mnemonic.validPhrase')}</span>
        </div>
      )}

      <Button
        type="button"
        variant="tertiary"
        onClick={() => switchMode('generate')}
        className="w-full mb-3">
        {t('mnemonic.generateNewPhrase')}
      </Button>
    </>
  );

  return (
    <SettingsPanel description={t('pages.settings.account.recoveryPhraseDesc')}>
      {success ? (
        <div className="flex flex-col items-center justify-center gap-3 py-12">
          <div className="w-12 h-12 rounded-full bg-sage-500/20 flex items-center justify-center">
            <svg
              className="w-6 h-6 text-sage-400"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
            </svg>
          </div>
          <p className="text-sm font-medium text-sage-500">{t('mnemonic.phraseSaved')}</p>
          <p className="text-xs text-content-muted">{t('mnemonic.walletReady')}</p>
        </div>
      ) : (
        <>
          {mode === 'loading' && renderLoading()}
          {mode === 'view' && renderViewMode()}
          {mode === 'replace-confirm' && renderReplaceConfirm()}
          {(mode === 'generate' || mode === 'import') && (
            <>
              {mode === 'generate' ? renderGenerateMode() : renderImportMode()}

              {error && (
                <div
                  role="alert"
                  className="flex items-start gap-2.5 p-3 mb-3 rounded-xl bg-coral-50 dark:bg-coral-500/10 border border-coral-200 dark:border-coral-500/30">
                  <svg
                    className="w-4 h-4 text-coral-500 flex-shrink-0 mt-0.5"
                    fill="none"
                    viewBox="0 0 24 24"
                    stroke="currentColor"
                    strokeWidth={2}>
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"
                    />
                  </svg>
                  <p className="text-xs text-coral-700 dark:text-coral-300 leading-relaxed">
                    {error}
                  </p>
                </div>
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
                    <svg className="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
                      <circle
                        className="opacity-25"
                        cx="12"
                        cy="12"
                        r="10"
                        stroke="currentColor"
                        strokeWidth="4"
                      />
                      <path
                        className="opacity-75"
                        fill="currentColor"
                        d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
                      />
                    </svg>
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
