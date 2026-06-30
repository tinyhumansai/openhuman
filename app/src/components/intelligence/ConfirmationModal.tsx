import { useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { ConfirmationModal as ConfirmationModalType } from '../../types/intelligence';
import Button from '../ui/Button';

interface ConfirmationModalProps {
  modal: ConfirmationModalType;
  onClose: () => void;
}

export function ConfirmationModal({ modal, onClose }: ConfirmationModalProps) {
  const { t } = useT();
  const [dontShowAgain, setDontShowAgain] = useState(false);

  if (!modal.isOpen) return null;

  const handleConfirm = () => {
    modal.onConfirm(dontShowAgain);
    onClose();

    if (dontShowAgain && modal.preferenceKey) {
      try {
        localStorage.setItem(modal.preferenceKey, 'true');
      } catch (err) {
        console.warn('Failed to save dontShowAgain preference to localStorage:', err);
      }
    }
  };

  const handleCancel = () => {
    modal.onCancel();
    onClose();
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/30 animate-fade-in"
      onClick={handleCancel}>
      <div
        className="bg-surface rounded-2xl max-w-md w-full shadow-large border border-line animate-slide-up"
        onClick={e => e.stopPropagation()}>
        {/* Header */}
        <div className="p-6 pb-4">
          <div className="flex items-center gap-3">
            {modal.destructive && (
              <div className="w-10 h-10 rounded-full bg-coral-50 dark:bg-coral-500/10 flex items-center justify-center flex-shrink-0">
                <svg
                  className="w-5 h-5 text-coral-400"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4c-.77-.833-1.964-.833-2.732 0L3.732 16.5c-.77.833.192 2.5 1.732 2.5z"
                  />
                </svg>
              </div>
            )}
            <div className="flex-1">
              <h2 className="text-lg font-semibold text-content">{modal.title}</h2>
              <p className="text-sm text-content-secondary mt-1">{modal.message}</p>
            </div>
          </div>
        </div>

        {/* Don't show again option */}
        {modal.showDontShowAgain && (
          <div className="px-6 pb-2">
            <label className="flex items-center gap-2 text-sm text-content-secondary cursor-pointer">
              <input
                type="checkbox"
                checked={dontShowAgain}
                onChange={e => setDontShowAgain(e.target.checked)}
                className="rounded border-line-strong bg-surface-subtle text-primary-500 focus:ring-primary-500 focus:ring-offset-0"
              />
              {t('modal.dontShowAgain')}
            </label>
          </div>
        )}

        {/* Actions */}
        <div className="flex items-center justify-end gap-3 p-6 pt-4 border-t border-line">
          <Button variant="tertiary" size="md" onClick={handleCancel}>
            {modal.cancelText || t('common.cancel')}
          </Button>
          <Button
            variant="primary"
            size="md"
            tone={modal.destructive ? 'danger' : 'default'}
            onClick={handleConfirm}>
            {modal.confirmText || t('common.confirm')}
          </Button>
        </div>
      </div>
    </div>
  );
}
