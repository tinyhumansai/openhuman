import { useEffect, useRef } from 'react';

import { type AccountProvider, type ProviderDescriptor, PROVIDERS } from '../../types/accounts';
import { ProviderIcon } from './providerIcons';

interface AddAccountModalProps {
  open: boolean;
  onClose: () => void;
  onPick: (provider: ProviderDescriptor) => void;
  /** Providers the user has already connected — filtered out of the picker. */
  connectedProviders?: ReadonlySet<AccountProvider>;
}

const AddAccountModal = ({ open, onClose, onPick, connectedProviders }: AddAccountModalProps) => {
  const closeBtnRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    closeBtnRef.current?.focus();
    return () => window.removeEventListener('keydown', onKey);
  }, [open, onClose]);

  if (!open) return null;

  const available = connectedProviders
    ? PROVIDERS.filter(p => !connectedProviders.has(p.id))
    : PROVIDERS;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-sm"
      role="dialog"
      aria-modal="true"
      onClick={onClose}>
      <div
        className="w-[420px] max-w-[90vw] rounded-2xl bg-white p-6 shadow-strong"
        onClick={e => e.stopPropagation()}>
        <div className="mb-4 flex items-center justify-between">
          <h2 className="text-lg font-semibold text-stone-900">Add account</h2>
          <button
            ref={closeBtnRef}
            onClick={onClose}
            className="rounded p-1 text-stone-500 hover:bg-stone-100"
            aria-label="Close">
            <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M6 18L18 6M6 6l12 12"
              />
            </svg>
          </button>
        </div>

        <div className="space-y-1">
          {available.length === 0 ? (
            <div className="rounded-lg border border-dashed border-stone-200 p-6 text-center text-sm text-stone-500">
              You've connected every supported app.
            </div>
          ) : (
            available.map(p => (
              <button
                key={p.id}
                onClick={() => onPick(p)}
                className="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left transition-colors hover:bg-stone-100">
                <ProviderIcon provider={p.id} className="h-5 w-5 flex-none" />
                <span className="text-sm font-medium text-stone-900">{p.label}</span>
              </button>
            ))
          )}
        </div>
      </div>
    </div>
  );
};

export default AddAccountModal;
