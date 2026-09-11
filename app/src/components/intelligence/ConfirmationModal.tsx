import { useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { ConfirmationModal as ConfirmationModalType } from '../../types/intelligence';
import {
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogRoot,
  AlertDialogTitle,
} from '../ui/AlertDialog';
import Checkbox from '../ui/Checkbox';
import Label from '../ui/Label';

interface ConfirmationModalProps {
  modal: ConfirmationModalType;
  onClose: () => void;
}

export function ConfirmationModal({ modal, onClose }: ConfirmationModalProps) {
  const { t } = useT();
  const [dontShowAgain, setDontShowAgain] = useState(false);
  // Radix closes an AlertDialog on both Cancel and Action clicks, firing the
  // same `onOpenChange(false)`. Confirm must not also run the cancel path, so
  // the confirm handler flags one close as "already handled" before it fires.
  const skipNextCancelRef = useRef(false);

  const handleConfirm = () => {
    skipNextCancelRef.current = true;
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
    <AlertDialogRoot
      open={modal.isOpen}
      onOpenChange={next => {
        if (next) return;
        if (skipNextCancelRef.current) {
          skipNextCancelRef.current = false;
          return;
        }
        handleCancel();
      }}>
      <AlertDialogContent className="max-w-md p-0">
        {/* Header */}
        <div className="p-6 pb-4">
          <div className="flex items-center gap-3">
            {modal.destructive && (
              <div className="w-10 h-10 rounded-full bg-coral-50 dark:bg-coral-500/10 flex items-center justify-center shrink-0">
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
              <AlertDialogTitle className="text-lg font-semibold">{modal.title}</AlertDialogTitle>
              <AlertDialogDescription className="mt-1">{modal.message}</AlertDialogDescription>
            </div>
          </div>
        </div>

        {/* Don't show again option */}
        {modal.showDontShowAgain && (
          <div className="px-6 pb-2">
            <Label
              htmlFor="confirmation-modal-dont-show-again"
              className="flex items-center gap-2 text-sm font-normal text-content-secondary cursor-pointer">
              <Checkbox
                id="confirmation-modal-dont-show-again"
                checked={dontShowAgain}
                onCheckedChange={setDontShowAgain}
              />
              {t('modal.dontShowAgain')}
            </Label>
          </div>
        )}

        {/* Actions */}
        <AlertDialogFooter className="p-6 pt-4 border-t border-line mt-0">
          {/* No onClick here: Radix's Cancel already closes the dialog, which
              routes through the Root's onOpenChange -> handleCancel above. */}
          <AlertDialogCancel>{modal.cancelText || t('common.cancel')}</AlertDialogCancel>
          <AlertDialogAction
            tone={modal.destructive ? 'danger' : 'default'}
            onClick={handleConfirm}>
            {modal.confirmText || t('common.confirm')}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialogRoot>
  );
}
