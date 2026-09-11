import { useT } from '../../../lib/i18n/I18nContext';
import type { WalletStatus } from '../../../services/walletApi';
import { Alert } from '../../ui/Alert';
import Button from '../../ui/Button';
import { CheckIcon, Spinner } from '../../ui/icons';

export interface RecoveryPhraseViewModeProps {
  statusError: string | null;
  walletStatus: WalletStatus | null;
  viewMnemonic: string | null;
  viewRevealed: boolean;
  onRevealBlur: () => void;
  onHide: () => void;
  viewRevealLoading: boolean;
  viewRevealError: string | null;
  onReveal: () => void;
  viewCopied: boolean;
  onCopy: () => void;
  onReplaceClick: () => void;
}

/**
 * View mode: the existing-wallet summary, its metadata (source, word count,
 * last updated, chain addresses), and the reveal/hide-existing-phrase flow.
 * Falls back to a single error alert when the initial status check failed.
 */
const RecoveryPhraseViewMode = ({
  statusError,
  walletStatus,
  viewMnemonic,
  viewRevealed,
  onRevealBlur,
  onHide,
  viewRevealLoading,
  viewRevealError,
  onReveal,
  viewCopied,
  onCopy,
  onReplaceClick,
}: RecoveryPhraseViewModeProps) => {
  const { t } = useT();

  if (statusError) {
    return (
      <div className="space-y-5">
        <Alert variant="destructive">
          <p className="text-xs leading-relaxed">{statusError}</p>
        </Alert>
      </div>
    );
  }

  return (
    <div className="space-y-5">
      <Alert variant="success">
        <p className="text-xs leading-relaxed font-medium">
          {t('mnemonic.walletAlreadyConfigured')}
        </p>
      </Alert>

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
              <span className="text-xs text-content-muted">{t('mnemonic.walletWordCount')}</span>
              <span className="text-xs font-medium text-content-secondary">
                {walletStatus.mnemonicWordCount} words
              </span>
            </div>
          )}
          {walletStatus.updatedAtMs && (
            <div className="flex items-center justify-between">
              <span className="text-xs text-content-muted">{t('mnemonic.walletLastUpdated')}</span>
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

      {viewMnemonic ? (
        <div className="space-y-3">
          <Alert variant="warning">
            <p className="text-xs leading-relaxed">{t('mnemonic.cannotRecover')}</p>
          </Alert>
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
              <Button
                type="button"
                variant="tertiary"
                iconOnly
                onClick={onRevealBlur}
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
            disabled={!viewRevealed}
            className="w-full">
            {viewCopied ? (
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
          <Button type="button" variant="tertiary" onClick={onHide} className="w-full">
            {t('mnemonic.hidePhrase')}
          </Button>
        </div>
      ) : (
        <>
          {viewRevealError && (
            <Alert variant="destructive">
              <p className="text-xs leading-relaxed">{viewRevealError}</p>
            </Alert>
          )}
          <Button
            type="button"
            variant="secondary"
            size="md"
            onClick={onReveal}
            disabled={viewRevealLoading}
            className="w-full">
            {viewRevealLoading ? (
              <>
                <Spinner className="w-4 h-4" />
                <span>{t('mnemonic.loadingWalletStatus')}</span>
              </>
            ) : (
              t('mnemonic.revealRecoveryPhrase')
            )}
          </Button>
        </>
      )}

      <Button
        type="button"
        variant="secondary"
        size="md"
        onClick={onReplaceClick}
        className="w-full">
        {t('mnemonic.replaceWallet')}
      </Button>
    </div>
  );
};

export default RecoveryPhraseViewMode;
