import { type KeyboardEvent, useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { useCoreState } from '../../../providers/CoreStateProvider';
import {
  deriveAesKeyFromMnemonic,
  generateMnemonicPhrase,
  MNEMONIC_GENERATE_WORD_COUNT,
  validateMnemonicPhrase,
} from '../../../utils/cryptoKeys';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const BIP39_IMPORT_LENGTHS = [12, 15, 18, 21, 24] as const;

const IMPORT_SLOTS_INITIAL = MNEMONIC_GENERATE_WORD_COUNT;

const RecoveryPhrasePanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const { snapshot, setEncryptionKey } = useCoreState();
  const user = snapshot.currentUser;

  const [mode, setMode] = useState<'generate' | 'import'>('generate');
  const [copied, setCopied] = useState(false);
  const [confirmed, setConfirmed] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState(false);

  const mnemonic = useMemo(() => generateMnemonicPhrase(), []);
  const words = useMemo(() => mnemonic.split(' '), [mnemonic]);

  const [selectedWordCount, setSelectedWordCount] = useState(IMPORT_SLOTS_INITIAL);
  const [importWords, setImportWords] = useState<string[]>(Array(IMPORT_SLOTS_INITIAL).fill(''));
  const [importValid, setImportValid] = useState<boolean | null>(null);
  const inputRefs = useRef<(HTMLInputElement | null)[]>([]);

  useEffect(() => {
    if (copied) {
      const timer = setTimeout(() => setCopied(false), 3000);
      return () => clearTimeout(timer);
    }
  }, [copied]);

  useEffect(() => {
    setConfirmed(false);
    setError(null);
    setImportValid(null);
    setSelectedWordCount(IMPORT_SLOTS_INITIAL);
    setImportWords(Array(IMPORT_SLOTS_INITIAL).fill(''));
  }, [mode]);

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

  // Navigate back after success
  useEffect(() => {
    if (success) {
      const timer = setTimeout(() => {
        navigateBack();
      }, 1500);
      return () => clearTimeout(timer);
    }
  }, [success, navigateBack]);

  const handleCopy = useCallback(async () => {
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
      setError('Invalid recovery phrase. Please check your words and try again.');
      return false;
    }

    setError(null);
    return true;
  }, [importWords]);

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
        phraseToUse = mnemonic;
      }

      const aesKey = deriveAesKeyFromMnemonic(phraseToUse);
      if (!user?._id) {
        setError('User not loaded. Please sign in again or refresh the page.');
        return;
      }
      await setEncryptionKey(aesKey);
      setSuccess(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Something went wrong. Please try again.');
    } finally {
      setLoading(false);
    }
  };

  const importWordCount = importWords.filter(w => w.trim()).length;
  const isImportComplete =
    importWords.every(w => w.trim()) &&
    BIP39_IMPORT_LENGTHS.includes(importWordCount as (typeof BIP39_IMPORT_LENGTHS)[number]);
  const canSave = mode === 'generate' ? confirmed : isImportComplete;

  return (
    <div>
      <SettingsHeader
        title="Recovery Phrase"
        showBackButton
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div>
        <div className="p-4">
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
              <p className="text-sm font-medium text-sage-500">Recovery phrase saved</p>
              <p className="text-xs text-stone-500">Returning to settings...</p>
            </div>
          ) : (
            <>
              {mode === 'generate' ? (
                <>
                  <div className="mb-4 space-y-3">
                    <p className="text-sm text-stone-600 leading-relaxed">
                      Write down these {MNEMONIC_GENERATE_WORD_COUNT} words in order and store them
                      somewhere safe. This phrase encrypts your data.
                    </p>
                    <div className="flex items-start gap-2.5 p-3 rounded-xl bg-amber-50 border border-amber-200/70">
                      <svg
                        className="w-4 h-4 text-amber-600 flex-shrink-0 mt-0.5"
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
                      <p className="text-xs text-amber-800 leading-relaxed">
                        This phrase can never be recovered if lost.
                      </p>
                    </div>
                  </div>

                  <div className="bg-stone-50 rounded-2xl p-4 mb-4 border border-stone-200">
                    <div className="grid grid-cols-3 gap-2">
                      {words.map((word, index) => (
                        <div
                          key={index}
                          className="flex items-center gap-2 bg-white rounded-lg px-3 py-2 text-sm border border-stone-200">
                          <span className="text-stone-500 font-mono text-xs w-5 text-right">
                            {index + 1}.
                          </span>
                          <span className="font-mono font-medium">{word}</span>
                        </div>
                      ))}
                    </div>
                  </div>

                  <button
                    onClick={handleCopy}
                    className="w-full flex items-center justify-center gap-2 border border-stone-200 hover:border-stone-300 font-medium py-2.5 text-sm rounded-xl text-stone-700 transition-all duration-200 mb-3">
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
                        <span className="text-sage-400">Copied to Clipboard</span>
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
                        <span>Copy to Clipboard</span>
                      </>
                    )}
                  </button>

                  <button
                    onClick={() => setMode('import')}
                    className="w-full text-center text-sm text-primary-400 hover:text-primary-600 transition-colors mb-3">
                    I already have a recovery phrase
                  </button>

                  <label className="flex items-start gap-3 cursor-pointer mb-4">
                    <input
                      type="checkbox"
                      checked={confirmed}
                      onChange={e => setConfirmed(e.target.checked)}
                      className="mt-0.5 w-4 h-4 rounded border-stone-500 text-primary-500 focus:ring-primary-500"
                    />
                    <span className="text-sm text-stone-700">
                      I have saved my recovery phrase in a safe place
                    </span>
                  </label>
                </>
              ) : (
                <>
                  <div className="mb-4">
                    <p className="text-sm text-stone-600 leading-relaxed">
                      Enter your recovery phrase below, or paste the full phrase into any field (12
                      words for new backups; 24-word phrases from older versions still work).
                    </p>
                  </div>

                  <div className="flex items-center gap-2 mb-3">
                    <span className="text-xs text-stone-500">Words:</span>
                    {BIP39_IMPORT_LENGTHS.map(len => (
                      <button
                        key={len}
                        type="button"
                        onClick={() => handleWordCountChange(len)}
                        className={`px-2.5 py-1 text-xs font-medium rounded-lg transition-colors ${
                          selectedWordCount === len
                            ? 'bg-primary-500/20 border-primary-500/40 text-primary-600 border'
                            : 'border border-stone-200 text-stone-500 hover:border-stone-300'
                        }`}>
                        {len}
                      </button>
                    ))}
                  </div>

                  <div className="bg-stone-50 rounded-2xl p-4 mb-4 border border-stone-200">
                    <div className="grid grid-cols-3 gap-2">
                      {importWords.map((word, index) => (
                        <div key={index} className="flex items-center gap-1.5">
                          <span className="text-stone-500 font-mono text-xs w-5 text-right shrink-0">
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
                            className={`w-full font-mono text-sm font-medium px-2 py-1.5 rounded-lg border bg-white text-stone-900 outline-none transition-colors ${
                              importValid === false && word.trim()
                                ? 'border-coral-400 focus:border-coral-300'
                                : importValid === true
                                  ? 'border-sage-400 focus:border-sage-300'
                                  : 'border-stone-200 focus:border-primary-400'
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
                      <span>Valid recovery phrase</span>
                    </div>
                  )}

                  <button
                    onClick={() => setMode('generate')}
                    className="w-full text-center text-sm text-primary-400 hover:text-primary-600 transition-colors mb-3">
                    Generate a new recovery phrase instead
                  </button>
                </>
              )}

              {error && (
                <div
                  role="alert"
                  className="flex items-start gap-2.5 p-3 mb-3 rounded-xl bg-coral-50 border border-coral-200/70">
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
                  <p className="text-xs text-coral-700 leading-relaxed">{error}</p>
                </div>
              )}

              <button
                type="button"
                onClick={() => void handleSave()}
                disabled={!canSave || loading}
                className="btn-primary w-full py-3 text-sm font-medium rounded-xl disabled:opacity-60 flex items-center justify-center gap-2">
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
                    <span>Securing Your Data...</span>
                  </>
                ) : (
                  'Save Recovery Phrase'
                )}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
};

export default RecoveryPhrasePanel;
