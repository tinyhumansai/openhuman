import { useCallback, useEffect, useRef, useState } from 'react';
import { LuCheck, LuChevronDown, LuCopy } from 'react-icons/lu';

import btcIcon from '../../../assets/icons/chains/bitcoin.svg';
import evmIcon from '../../../assets/icons/chains/evm.svg';
import solanaIcon from '../../../assets/icons/chains/solana.svg';
import tronIcon from '../../../assets/icons/chains/trx.svg';
import { useT } from '../../../lib/i18n/I18nContext';
import { intlLocale } from '../../../lib/i18n/intlLocale';
import { revealRecoveryPhrase, type WalletStatus } from '../../../services/walletApi';
import { Alert } from '../../ui/Alert';
import Button from '../../ui/Button';
import {
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuRoot,
  DropdownMenuTrigger,
} from '../../ui/DropdownMenu';
import { CheckIcon, Spinner } from '../../ui/icons';
import RevealRecoveryModal from './wallet/RevealRecoveryModal';

const CHAIN_ICONS: Record<string, string> = {
  evm: evmIcon,
  btc: btcIcon,
  solana: solanaIcon,
  tron: tronIcon,
};

export interface RecoveryPhraseViewModeProps {
  statusError: string | null;
  walletStatus: WalletStatus | null;
  onGenerateClick: () => void;
  onImportClick: () => void;
}

// view mode: the existing-wallet summary. falls back to an error alert when the initial status check failed.
const RecoveryPhraseViewMode = ({
  statusError,
  walletStatus,
  onGenerateClick,
  onImportClick,
}: RecoveryPhraseViewModeProps) => {
  const { t, locale } = useT();
  const [isModalOpen, setIsModalOpen] = useState(false);
  const [mnemonic, setMnemonic] = useState<string | null>(null);
  const [isRevealing, setIsRevealing] = useState(false);
  const [revealError, setRevealError] = useState<string | null>(null);
  const [copiedChain, setCopiedChain] = useState<string | null>(null);

  const cleanupTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const inactivityTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    return () => {
      if (cleanupTimerRef.current) clearTimeout(cleanupTimerRef.current);
      if (inactivityTimerRef.current) clearTimeout(inactivityTimerRef.current);
      setMnemonic(null);
    };
  }, []);

  const resetInactivityTimer = useCallback(() => {
    if (inactivityTimerRef.current) clearTimeout(inactivityTimerRef.current);
    if (isModalOpen && mnemonic) {
      inactivityTimerRef.current = setTimeout(() => {
        setIsModalOpen(false);
        setMnemonic(null);
      }, 60000);
    }
  }, [isModalOpen, mnemonic]);

  useEffect(() => {
    if (isModalOpen && mnemonic) {
      resetInactivityTimer();
      const events = ['mousemove', 'keydown', 'touchstart', 'scroll', 'click'];
      events.forEach(event => window.addEventListener(event, resetInactivityTimer));
      return () => {
        events.forEach(event => window.removeEventListener(event, resetInactivityTimer));
        if (inactivityTimerRef.current) clearTimeout(inactivityTimerRef.current);
      };
    }
  }, [isModalOpen, mnemonic, resetInactivityTimer]);

  const handleCopy = async (chain: string, address: string) => {
    try {
      await navigator.clipboard.writeText(address);
      setCopiedChain(chain);
      setTimeout(() => setCopiedChain(null), 2000);
    } catch {
      // ignore
    }
  };

  // Fetch seed phrase on reveal
  const handleRevealClick = async () => {
    if (cleanupTimerRef.current) clearTimeout(cleanupTimerRef.current);
    setIsRevealing(true);
    setRevealError(null);
    try {
      const result = await revealRecoveryPhrase();
      setMnemonic(result.phrase);
      setIsModalOpen(true);
    } catch (e) {
      setRevealError(e instanceof Error ? e.message : 'Failed to retrieve phrase');
    } finally {
      setIsRevealing(false);
    }
  };

  if (statusError) {
    return (
      <div className="space-y-5">
        <Alert variant="destructive" className="border-0 bg-destructive/10 p-4">
          <p className="text-sm leading-relaxed text-destructive">{statusError}</p>
        </Alert>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div className="inline-flex items-center gap-2 bg-emerald-500/10 rounded-xl px-3.5 py-1.5 w-fit">
        <CheckIcon className="w-4 h-4 text-emerald-500" />
        <p className="text-sm font-medium text-emerald-500">
          {t('mnemonic.walletAlreadyConfigured')}
        </p>
      </div>

      {walletStatus && (
        <div className="space-y-6">
          <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
            {walletStatus.source && (
              <div className="bg-surface-muted/30 rounded-[14px] p-4 border border-line flex flex-col justify-center">
                <span className="text-xs text-content-muted mb-1">
                  {t('mnemonic.walletSource')}
                </span>
                <span className="text-sm font-medium text-content capitalize">
                  {walletStatus.source}
                </span>
              </div>
            )}
            {walletStatus.mnemonicWordCount && (
              <div className="bg-surface-muted/30 rounded-[14px] p-4 border border-line flex flex-col justify-center">
                <span className="text-xs text-content-muted mb-1">
                  {t('mnemonic.walletWordCount')}
                </span>
                <span className="text-sm font-medium text-content">
                  {walletStatus.mnemonicWordCount} {t('mnemonic.words')}
                </span>
              </div>
            )}
            {walletStatus.updatedAtMs && (
              <div className="bg-surface-muted/30 rounded-[14px] p-4 border border-line flex flex-col justify-center">
                <span className="text-xs text-content-muted mb-1">
                  {t('mnemonic.walletLastUpdated')}
                </span>
                <span className="text-sm font-medium text-content">
                  {new Date(walletStatus.updatedAtMs).toLocaleDateString(intlLocale(locale))}
                </span>
              </div>
            )}
          </div>

          {walletStatus.accounts.length > 0 && (
            <div className="space-y-3">
              <span className="text-xs font-semibold text-content block px-1">
                {t('mnemonic.viewAccounts')}
              </span>
              <div className="rounded-[14px] border border-line bg-surface overflow-hidden">
                {walletStatus.accounts.map((account, idx) => (
                  <div
                    key={account.chain}
                    className={`flex items-center justify-between gap-3 py-3 px-3.5 hover:bg-surface-hover/50 transition-colors group ${idx !== walletStatus.accounts.length - 1 ? 'border-b border-line/50' : ''}`}>
                    <div className="flex items-center gap-3">
                      <div className="w-9 flex items-center justify-center shrink-0">
                        {CHAIN_ICONS[account.chain] && (
                          <img
                            src={CHAIN_ICONS[account.chain]}
                            alt={account.chain}
                            className={
                              account.chain === 'evm'
                                ? 'w-9 h-9 shrink-0 object-contain'
                                : 'w-7 h-7 shrink-0 object-contain'
                            }
                          />
                        )}
                      </div>
                      <span className="text-sm font-bold text-content font-mono uppercase">
                        {account.chain}
                      </span>
                    </div>
                    <div className="flex items-center gap-2">
                      <span className="text-[13px] font-mono text-content-muted break-all">
                        {account.address}
                      </span>
                      <button
                        onClick={() => handleCopy(account.chain, account.address)}
                        className="p-1.5 rounded-md hover:bg-surface-muted text-content-muted transition-colors"
                        title={t('walletBalances.copyAddress')}>
                        {copiedChain === account.chain ? (
                          <LuCheck className="w-3.5 h-3.5 text-emerald-500" />
                        ) : (
                          <LuCopy className="w-3.5 h-3.5" />
                        )}
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>
      )}

      {revealError && (
        <Alert variant="destructive" className="border-0 bg-destructive/10 p-4">
          <p className="text-sm leading-relaxed text-destructive">{revealError}</p>
        </Alert>
      )}

      <div className="space-y-3">
        <Button
          type="button"
          variant="primary"
          size="md"
          onClick={handleRevealClick}
          disabled={isRevealing}
          className="w-full font-semibold">
          {isRevealing ? (
            <>
              <Spinner className="w-4 h-4" />
              <span>{t('common.loading')}</span>
            </>
          ) : (
            t('mnemonic.revealRecoveryPhrase')
          )}
        </Button>

        <DropdownMenuRoot>
          <DropdownMenuTrigger asChild>
            <Button
              type="button"
              variant="tertiary"
              size="md"
              className="w-full bg-surface dark:bg-content-inverted/5 border border-line hover:bg-surface-hover dark:hover:bg-content-inverted/10 group transition-all duration-200">
              <span className="text-content-secondary group-hover:text-content font-semibold transition-colors flex items-center justify-center gap-2">
                {t('mnemonic.replaceWallet')}
                <LuChevronDown className="w-4 h-4" />
              </span>
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent className="w-64" align="center">
            <DropdownMenuItem onClick={onGenerateClick}>
              {t('mnemonic.createANewWallet')}
            </DropdownMenuItem>
            <DropdownMenuItem onClick={onImportClick}>
              {t('mnemonic.importAnExistingWallet')}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenuRoot>
      </div>

      {isModalOpen && mnemonic && (
        <RevealRecoveryModal
          open={isModalOpen}
          onClose={() => {
            setIsModalOpen(false);
            if (cleanupTimerRef.current) clearTimeout(cleanupTimerRef.current);
            cleanupTimerRef.current = setTimeout(() => setMnemonic(null), 300);
          }}
          mnemonic={mnemonic}
        />
      )}
    </div>
  );
};

export default RecoveryPhraseViewMode;
