import { useCallback, useRef } from 'react';

import {
  openhumanLocalAiDownload,
  openhumanLocalAiDownloadAllAssets,
} from '../../../utils/tauriCommands';

/* ---------- component ---------- */

interface LocalAIStepProps {
  onNext: (result: { consentGiven: boolean; downloadStarted: boolean }) => void;
  onBack?: () => void;
}

const LocalAIStep = ({ onNext, onBack }: LocalAIStepProps) => {
  const downloadStartedRef = useRef(false);

  const handleConsent = useCallback(() => {
    if (downloadStartedRef.current) return;
    downloadStartedRef.current = true;

    // Fire-and-forget: start downloads in the background — the global snackbar tracks progress
    void openhumanLocalAiDownload(false).catch(() => {});
    void openhumanLocalAiDownloadAllAssets(false).catch(() => {});

    // Advance to next step immediately
    onNext({ consentGiven: true, downloadStarted: true });
  }, [onNext]);

  const handleSkip = useCallback(() => {
    onNext({ consentGiven: false, downloadStarted: false });
  }, [onNext]);

  return (
    <div className="rounded-3xl border border-stone-700 bg-stone-900 p-8 shadow-large animate-fade-up">
      <div className="flex flex-col items-center mb-5">
        <img src="/ollama.svg" alt="Ollama" className="w-16 h-16 mb-3" />
        <h1 className="text-xl font-bold mb-2">Run AI Models Locally with Ollama</h1>
        <p className="opacity-70 text-sm text-center">
          OpenHuman will auto-install Ollama for you so that you can download and run AI models
          locally on your device.
        </p>
      </div>

      <div className="space-y-2 mb-5">
        <div className="rounded-xl border border-sage-500/30 bg-sage-500/10 px-3 py-2">
          <p className="text-xs">
            <span className="font-semibold">Complete Privacy</span>
            <span className="opacity-80">
              &nbsp;- all data stays on your device. Nothing is sent to any third party or cloud.
            </span>
          </p>
        </div>
        <div className="rounded-xl border border-sage-500/30 bg-sage-500/10 px-3 py-2">
          <p className="text-xs">
            <span className="font-semibold">Absolutely Free</span>
            <span className="opacity-80">
              &nbsp;- Ollama and the AI models are open-source. No subscription needed.
            </span>
          </p>
        </div>
        <div className="rounded-xl border border-amber-500/30 bg-amber-500/10 px-3 py-2">
          <p className="text-xs">
            <span className="font-semibold">Resource impact</span>
            <span className="opacity-80">
              &nbsp;- uses some disk space and RAM. We will optimize this for your device.
            </span>
          </p>
        </div>
      </div>

      <button
        onClick={handleConsent}
        className="w-full py-2.5 btn-primary text-sm font-medium rounded-xl border transition-colors border-stone-600 hover:border-sage-500 hover:bg-sage-500/10 mb-3">
        Use Local Models
      </button>

      <div className="flex gap-2">
        {onBack && (
          <button
            onClick={onBack}
            className="py-2.5 px-4 text-sm font-medium rounded-xl bg-stone-800 hover:bg-stone-700 transition-colors">
            Back
          </button>
        )}
        <button
          onClick={handleSkip}
          className="flex-1 py-2.5 text-sm font-medium rounded-xl bg-stone-800 hover:bg-stone-700 transition-colors">
          Setup Later
        </button>
      </div>
    </div>
  );
};

export default LocalAIStep;
