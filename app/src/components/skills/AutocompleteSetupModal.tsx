/**
 * Text Auto-Complete setup/enable modal.
 *
 * Simple enable flow: shows current state, lets user enable with one click,
 * and shows a success confirmation — matching the UX of the Screen
 * Intelligence setup modal.
 */
import { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { useNavigate } from 'react-router-dom';

import { useCoreState } from '../../providers/CoreStateProvider';
import {
  openhumanAutocompleteSetStyle,
  openhumanAutocompleteStart,
} from '../../utils/tauriCommands/autocomplete';

type Step = 'enable' | 'success';

interface Props {
  onClose: () => void;
}

export default function AutocompleteSetupModal({ onClose }: Props) {
  const navigate = useNavigate();
  const { snapshot, refresh } = useCoreState();
  const status = snapshot.runtime.autocomplete;

  const [step, setStep] = useState<Step>('enable');
  const [isEnabling, setIsEnabling] = useState(false);
  const [enableError, setEnableError] = useState<string | null>(null);

  // Close on Escape key
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [onClose]);

  const handleEnable = async () => {
    setIsEnabling(true);
    setEnableError(null);
    try {
      // Enable in config
      await openhumanAutocompleteSetStyle({ enabled: true });
      // Start the service
      await openhumanAutocompleteStart();
      await refresh();
      setStep('success');
    } catch (error) {
      setEnableError(error instanceof Error ? error.message : 'Failed to enable autocomplete');
    } finally {
      setIsEnabling(false);
    }
  };

  const handleGoToSettings = () => {
    onClose();
    navigate('/settings/autocomplete');
  };

  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
      onClick={e => {
        if (e.target === e.currentTarget) onClose();
      }}>
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="ac-setup-title"
        className="w-full max-w-md mx-4 rounded-2xl bg-white shadow-xl overflow-hidden animate-fade-up">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-stone-100 px-5 py-4">
          <div className="flex items-center gap-3">
            <div className="w-9 h-9 rounded-xl bg-primary-50 flex items-center justify-center text-primary-600">
              <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={1.8}
                  d="M4 7h16M4 12h10m-10 5h7m10 0l3 3m0 0l3-3m-3 3v-8"
                />
              </svg>
            </div>
            <div>
              <h2 id="ac-setup-title" className="text-sm font-semibold text-stone-900">Text Auto-Complete</h2>
              <p className="text-xs text-stone-500">
                {step === 'enable' && 'Enable inline completions'}
                {step === 'success' && 'Ready to go'}
              </p>
            </div>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="w-7 h-7 rounded-lg flex items-center justify-center text-stone-400 hover:text-stone-600 hover:bg-stone-100 transition-colors">
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Body */}
        <div className="px-5 py-4">
          {/* ─── Enable step ─── */}
          {step === 'enable' && (
            <div className="space-y-4">
              <p className="text-xs text-stone-500 leading-relaxed">
                Text Auto-Complete suggests inline completions as you type across any app. Suggestions appear as an overlay you can accept with Tab.
              </p>

              {!status?.platform_supported && status !== null && (
                <div className="rounded-xl border border-amber-200 bg-amber-50 p-3 text-xs text-amber-700">
                  Auto-complete is not supported on this platform.
                </div>
              )}

              <div className="space-y-2">
                <div className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2.5">
                  <span className="text-sm text-stone-700">Style preset</span>
                  <span className="text-xs text-stone-500">Balanced (configurable later)</span>
                </div>
                <div className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2.5">
                  <span className="text-sm text-stone-700">Accept key</span>
                  <span className="text-xs font-mono text-stone-500">Tab</span>
                </div>
                <div className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2.5">
                  <span className="text-sm text-stone-700">Debounce</span>
                  <span className="text-xs text-stone-500">{status?.debounce_ms ?? 120}ms</span>
                </div>
              </div>

              {enableError && (
                <div className="rounded-xl border border-coral-200 bg-coral-50 p-3 text-xs text-coral-700">
                  {enableError}
                </div>
              )}

              <button
                type="button"
                onClick={() => void handleEnable()}
                disabled={isEnabling || (status !== null && !status.platform_supported)}
                className="w-full rounded-xl bg-primary-500 px-4 py-2.5 text-sm font-medium text-white hover:bg-primary-600 disabled:opacity-50 transition-colors">
                {isEnabling ? 'Enabling...' : 'Enable Auto-Complete'}
              </button>
            </div>
          )}

          {/* ─── Success step ─── */}
          {step === 'success' && (
            <div className="space-y-4 text-center py-2">
              <div className="mx-auto w-12 h-12 rounded-full bg-sage-50 flex items-center justify-center">
                <svg className="w-6 h-6 text-sage-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                </svg>
              </div>

              <div>
                <h3 className="text-sm font-semibold text-stone-900">Auto-Complete is Active</h3>
                <p className="mt-1 text-xs text-stone-500 leading-relaxed">
                  Start typing in any app and suggestions will appear as an inline overlay. Press Tab to accept.
                </p>
              </div>

              <div className="flex flex-col gap-2">
                <button
                  type="button"
                  onClick={handleGoToSettings}
                  className="w-full rounded-xl border border-primary-200 bg-primary-50 px-4 py-2.5 text-sm font-medium text-primary-700 hover:bg-primary-100 transition-colors">
                  Customize Settings
                </button>
                <button
                  type="button"
                  onClick={onClose}
                  className="w-full rounded-xl border border-stone-200 bg-stone-50 px-4 py-2.5 text-sm font-medium text-stone-600 hover:bg-stone-100 transition-colors">
                  Done
                </button>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>,
    document.body
  );
}
