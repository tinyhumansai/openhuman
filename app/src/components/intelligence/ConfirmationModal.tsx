import { useState } from 'react';

import type { ConfirmationModal as ConfirmationModalType } from '../../types/intelligence';

interface ConfirmationModalProps {
  modal: ConfirmationModalType;
  onClose: () => void;
}

export function ConfirmationModal({ modal, onClose }: ConfirmationModalProps) {
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
        className="bg-white rounded-2xl max-w-md w-full shadow-large border border-stone-200 animate-slide-up"
        onClick={e => e.stopPropagation()}>
        {/* Header */}
        <div className="p-6 pb-4">
          <div className="flex items-center gap-3">
            {modal.destructive && (
              <div className="w-10 h-10 rounded-full bg-coral-500/10 flex items-center justify-center flex-shrink-0">
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
              <h2 className="text-lg font-semibold text-stone-900">{modal.title}</h2>
              <p className="text-sm text-stone-600 mt-1">{modal.message}</p>
            </div>
          </div>
        </div>

        {/* Don't show again option */}
        {modal.showDontShowAgain && (
          <div className="px-6 pb-2">
            <label className="flex items-center gap-2 text-sm text-stone-600 cursor-pointer">
              <input
                type="checkbox"
                checked={dontShowAgain}
                onChange={e => setDontShowAgain(e.target.checked)}
                className="rounded border-stone-300 bg-stone-100 text-primary-500 focus:ring-primary-500 focus:ring-offset-0"
              />
              Don't show similar suggestions
            </label>
          </div>
        )}

        {/* Actions */}
        <div className="flex items-center justify-end gap-3 p-6 pt-4 border-t border-stone-200">
          <button
            onClick={handleCancel}
            className="px-4 py-2 text-sm font-medium text-stone-600 hover:text-stone-900 rounded-lg hover:bg-stone-100 transition-colors">
            {modal.cancelText || 'Cancel'}
          </button>
          <button
            onClick={handleConfirm}
            className={`
              px-4 py-2 text-sm font-medium rounded-lg transition-colors
              ${
                modal.destructive
                  ? 'bg-coral-500 hover:bg-coral-600 text-white'
                  : 'bg-primary-500 hover:bg-primary-600 text-white'
              }
            `}>
            {modal.confirmText || 'Confirm'}
          </button>
        </div>
      </div>
    </div>
  );
}
