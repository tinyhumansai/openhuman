import { useT } from '../../../lib/i18n/I18nContext';
import { MNEMONIC_GENERATE_WORD_COUNT } from '../../../utils/cryptoKeys';
import { Alert } from '../../ui/Alert';
import Button from '../../ui/Button';
import { CheckIcon } from '../../ui/icons';
import { SettingsCheckbox } from '../controls';

export interface RecoveryPhraseGenerateModeProps {
  words: string[];
  revealed: boolean;
  onReveal: () => void;
  copied: boolean;
  onCopy: () => void;
  confirmed: boolean;
  onConfirmedChange: (next: boolean) => void;
  onSwitchToImport: () => void;
}

/**
 * Generate-mode body: the newly generated word grid (blurred until revealed),
 * the copy-to-clipboard action, and the consent checkbox that gates Save.
 */
const RecoveryPhraseGenerateMode = ({
  words,
  revealed,
  onReveal,
  copied,
  onCopy,
  confirmed,
  onConfirmedChange,
  onSwitchToImport,
}: RecoveryPhraseGenerateModeProps) => {
  const { t } = useT();

  return (
    <>
      <div className="mb-4 space-y-3">
        <p className="text-sm text-content-secondary leading-relaxed">
          {t('mnemonic.writeDownWords')} {MNEMONIC_GENERATE_WORD_COUNT} {t('mnemonic.wordsInOrder')}
        </p>
        <Alert variant="warning">
          <p className="text-xs leading-relaxed">{t('mnemonic.cannotRecover')}</p>
        </Alert>
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
          <Button
            type="button"
            variant="tertiary"
            iconOnly
            onClick={onReveal}
            aria-label={t('mnemonic.revealPhrase')}
            className="absolute inset-0 h-auto w-auto rounded-none bg-transparent hover:bg-transparent focus-visible:ring-offset-0">
            <svg
              className="w-7 h-7 text-content transition-opacity duration-200 hover:opacity-70"
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
          </Button>
        )}
      </div>

      <Button
        type="button"
        variant="secondary"
        size="md"
        onClick={onCopy}
        disabled={!revealed}
        className="w-full mb-3">
        {copied ? (
          <>
            <CheckIcon className="w-4 h-4 text-sage-400" />
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

      <Button type="button" variant="tertiary" onClick={onSwitchToImport} className="w-full mb-3">
        {t('mnemonic.alreadyHavePhrase')}
      </Button>

      <label className="flex items-start gap-3 cursor-pointer mb-4">
        <SettingsCheckbox
          id="mnemonic-confirm-checkbox"
          checked={confirmed}
          onCheckedChange={onConfirmedChange}
        />
        <span className="text-sm text-content-secondary">{t('mnemonic.consentSaved')}</span>
      </label>
    </>
  );
};

export default RecoveryPhraseGenerateMode;
