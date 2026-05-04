import { useEffect, useRef } from 'react';
import { createPortal } from 'react-dom';

import Button from '../../components/ui/Button';
import { WHAT_LEAVES_HEADLINE, WHAT_LEAVES_ITEMS, WHAT_LEAVES_SUBHEAD } from './whatLeavesItems';

export interface WhatLeavesMyComputerSheetProps {
  open: boolean;
  onClose: () => void;
}

const WhatLeavesMyComputerSheet = ({ open, onClose }: WhatLeavesMyComputerSheetProps) => {
  const dialogRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', onKey);
    dialogRef.current?.focus();
    return () => document.removeEventListener('keydown', onKey);
  }, [open, onClose]);

  if (!open) return null;

  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4 animate-fade-in"
      role="presentation">
      <button
        type="button"
        aria-label="Close"
        onClick={onClose}
        className="absolute inset-0 bg-neutral-900/40 backdrop-blur-sm cursor-default"
      />
      <div
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby="what-leaves-title"
        className="relative w-full max-w-lg bg-neutral-0 rounded-2xl shadow-large border border-neutral-200 p-6 animate-fade-up outline-none">
        <div className="flex items-start justify-between gap-4 mb-4">
          <div>
            <h2
              id="what-leaves-title"
              className="font-display text-2xl text-neutral-900 leading-tight">
              {WHAT_LEAVES_HEADLINE}
            </h2>
          </div>
        </div>
        <p className="text-sm text-neutral-600 mb-5 max-w-md">{WHAT_LEAVES_SUBHEAD}</p>

        <ul className="space-y-3 mb-6">
          {WHAT_LEAVES_ITEMS.map(item => (
            <li key={item.id} className="rounded-xl border border-neutral-200 bg-neutral-50 p-4">
              <p className="text-sm font-medium text-neutral-900">{item.title}</p>
              <p className="text-sm text-neutral-600 mt-1 leading-relaxed">{item.body}</p>
            </li>
          ))}
        </ul>

        <div className="flex items-center justify-between gap-3">
          <p className="text-xs text-neutral-500">
            Full controls live in Settings → Privacy & Security.
          </p>
          <Button variant="primary" size="md" onClick={onClose}>
            Got it
          </Button>
        </div>
      </div>
    </div>,
    document.body
  );
};

export default WhatLeavesMyComputerSheet;
