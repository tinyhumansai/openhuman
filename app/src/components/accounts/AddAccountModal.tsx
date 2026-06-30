import { useEffect, useRef } from 'react';

import { useEscapeKey } from '../../hooks/useEscapeKey';
import { useT } from '../../lib/i18n/I18nContext';
import { type AccountProvider, type ProviderDescriptor, PROVIDERS } from '../../types/accounts';
import { CloseIcon } from '../ui';
import Button from '../ui/Button';
import { ProviderIcon } from './providerIcons';

interface AddAccountModalProps {
  open: boolean;
  onClose: () => void;
  onPick: (provider: ProviderDescriptor) => void;
  /** Providers the user has already connected — filtered out of the picker. */
  connectedProviders?: ReadonlySet<AccountProvider>;
}

const AddAccountModal = ({ open, onClose, onPick, connectedProviders }: AddAccountModalProps) => {
  const { t } = useT();
  const closeBtnRef = useRef<HTMLButtonElement | null>(null);

  useEscapeKey(onClose, open);

  useEffect(() => {
    if (!open) return;
    closeBtnRef.current?.focus();
  }, [open]);
  if (!open) return null;

  const available = connectedProviders
    ? PROVIDERS.filter(p => !connectedProviders.has(p.id))
    : PROVIDERS;

  return (
    <div
      data-testid="add-account-modal"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-sm"
      role="dialog"
      aria-modal="true"
      aria-labelledby="add-account-modal-title"
      onClick={onClose}>
      <div
        className="w-[420px] max-w-[90vw] rounded-2xl bg-surface p-6 shadow-strong"
        onClick={e => e.stopPropagation()}>
        <div className="mb-4 flex items-center justify-between">
          <h2 id="add-account-modal-title" className="text-lg font-semibold text-content">
            {t('accounts.addModal.title')}
          </h2>
          <Button
            ref={closeBtnRef}
            iconOnly
            variant="tertiary"
            size="sm"
            onClick={onClose}
            data-analytics-id="add-account-modal-close"
            aria-label={t('common.close')}>
            <CloseIcon className="h-5 w-5" />
          </Button>
        </div>

        <div className="space-y-1">
          {available.length === 0 ? (
            <div className="rounded-lg border border-dashed border-line p-6 text-center text-sm text-content-muted">
              {t('accounts.addModal.allConnected')}
            </div>
          ) : (
            available.map(p => (
              <button
                key={p.id}
                type="button"
                data-analytics-id={`add-account-provider-${p.id}`}
                data-testid={`add-account-provider-${p.id}`}
                onClick={() => onPick(p)}
                className="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left transition-colors hover:bg-surface-hover dark:bg-surface-muted dark:hover:bg-surface-muted/60">
                <ProviderIcon provider={p.id} className="h-5 w-5 flex-none" />
                <span className="text-sm font-medium text-content">{p.label}</span>
              </button>
            ))
          )}
        </div>
      </div>
    </div>
  );
};

export default AddAccountModal;
