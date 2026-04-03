import { useCallback, useRef } from 'react';

import {
  openhumanLocalAiDownload,
  openhumanLocalAiDownloadAllAssets,
} from '../../../utils/tauriCommands';
import OnboardingNextButton from '../components/OnboardingNextButton';

/* ---------- component ---------- */

interface LocalAIStepProps {
  onNext: (result: { consentGiven: boolean; downloadStarted: boolean }) => void;
  onBack?: () => void;
  onDownloadError?: (message: string) => void;
}

const LocalAIStep = ({ onNext, onBack: _onBack, onDownloadError }: LocalAIStepProps) => {
  const downloadStartedRef = useRef(false);
  // Tracks whether onDownloadError has already been called for this download attempt,
  // so that two concurrent failures don't fire the callback twice.
  const errorReportedRef = useRef(false);

  const handleConsent = useCallback(() => {
    if (downloadStartedRef.current) return;
    downloadStartedRef.current = true;
    errorReportedRef.current = false;

    const reportError = (source: string, err: unknown) => {
      console.warn(`[LocalAIStep] Download failed (${source}):`, err);
      if (!errorReportedRef.current) {
        errorReportedRef.current = true;
        onDownloadError?.('Local AI setup encountered an issue');
      }
    };

    // Fire-and-forget: start downloads in the background — the global snackbar tracks progress
    void openhumanLocalAiDownload(false).catch((err: unknown) => reportError('ollama', err));
    void openhumanLocalAiDownloadAllAssets(false).catch((err: unknown) =>
      reportError('assets', err)
    );

    // Advance to next step immediately
    onNext({ consentGiven: true, downloadStarted: true });
  }, [onNext, onDownloadError]);

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

      <OnboardingNextButton label="Continue" onClick={handleConsent} />
    </div>
  );
};

export default LocalAIStep;
