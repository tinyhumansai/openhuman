import { useCallback, useRef, useState } from 'react';

import {
  openhumanLocalAiDownload,
  openhumanLocalAiDownloadAllAssets,
} from '../../../utils/tauriCommands';

/* ---------- component ---------- */

interface LocalAIStepProps {
  onNext: (result: { consentGiven: boolean; downloadStarted: boolean }) => void;
}

const LocalAIStep = ({ onNext }: LocalAIStepProps) => {
  const [consent, setConsent] = useState<boolean | null>(null);
  const downloadStartedRef = useRef(false);

  const handleConsent = useCallback(() => {
    if (downloadStartedRef.current) return;
    downloadStartedRef.current = true;
    setConsent(true);

    // Fire-and-forget: start downloads in the background — the global snackbar tracks progress
    void openhumanLocalAiDownload(false).catch(() => {});
    void openhumanLocalAiDownloadAllAssets(false).catch(() => {});

    // Advance to next step immediately
    onNext({ consentGiven: true, downloadStarted: true });
  }, [onNext]);

  /* ---------- Phase 1: consent ---------- */
  if (consent === null) {
    return (
      <div className="rounded-3xl border border-stone-700 bg-stone-900 p-8 shadow-large animate-fade-up">
        <div className="text-center mb-5">
          <h1 className="text-xl font-bold mb-2">Download Local AI Models</h1>
          <p className="opacity-70 text-sm">
            OpenHuman uses local AI models directly on your device for faster, more private
            assistance. You can always change this later in Settings.
          </p>
        </div>

        <div className="space-y-3 mb-5">
          <div className="rounded-2xl border border-sage-500/30 bg-sage-500/10 p-3">
            <p className="text-sm font-medium mb-1">Complete Privacy</p>
            <p className="text-xs opacity-80">
              All your private & sensitive data gets processed locally by your local AI model. No
              data is sent to any third party.
            </p>
          </div>
          <div className="rounded-2xl border border-sage-500/30 bg-sage-500/10 p-3">
            <p className="text-sm font-medium mb-1">Absolutely Free</p>
            <p className="text-xs opacity-80">
              Running local AI models is free and does not require any subscription or payment.
            </p>
          </div>
          <div className="rounded-2xl border border-amber-500/30 bg-amber-500/10 p-3">
            <p className="text-sm font-medium mb-1">Resource impact</p>
            <p className="text-xs opacity-80">
              Typical setup needs 1-3 GB disk for model files and can use 1-2 GB RAM while running.
            </p>
          </div>
        </div>

        <div className="grid grid-cols-2 gap-2 mb-4">
          <button
            onClick={() => setConsent(false)}
            className="py-2.5 text-sm font-medium rounded-xl border transition-colors border-stone-600 hover:border-stone-500">
            Skip
          </button>
          <button
            onClick={handleConsent}
            className="py-2.5 btn-primary text-sm font-medium rounded-xl border transition-colors border-stone-600 hover:border-sage-500 hover:bg-sage-500/10">
            Download Local Models
          </button>
        </div>
      </div>
    );
  }

  /* ---------- Phase 2: consent=false, skip ---------- */
  if (consent === false) {
    return (
      <div className="rounded-3xl border border-stone-700 bg-stone-900 p-8 shadow-large animate-fade-up">
        <div className="text-center mb-5">
          <h1 className="text-xl font-bold mb-2">Local AI Models</h1>
          <p className="opacity-70 text-sm">
            No worries — you can always enable local models later in Settings.
          </p>
        </div>
        <button
          onClick={() => onNext({ consentGiven: false, downloadStarted: false })}
          className="btn-primary w-full py-2.5 text-sm font-medium rounded-xl">
          Continue
        </button>
      </div>
    );
  }

  /* consent=true triggers download + advance via handleConsent — render nothing */
  return null;
};

export default LocalAIStep;
