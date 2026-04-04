import { useCallback, useRef } from 'react';

import { bootstrapLocalAiWithRecommendedPreset } from '../../../utils/localAiBootstrap';
import OnboardingNextButton from '../components/OnboardingNextButton';

/* ---------- component ---------- */

interface LocalAIStepProps {
  onNext: (result: { consentGiven: boolean; downloadStarted: boolean }) => void;
  onBack?: () => void;
  onDownloadError?: (message: string) => void;
}

const LocalAIStep = ({ onNext, onBack: _onBack, onDownloadError }: LocalAIStepProps) => {
  const downloadStartedRef = useRef(false);

  const handleConsent = useCallback(() => {
    if (downloadStartedRef.current) return;
    downloadStartedRef.current = true;
    console.debug('[LocalAIStep] starting background Local AI bootstrap after consent');

    // Fire-and-forget: start bootstrap in the background — the global snackbar tracks progress.
    void bootstrapLocalAiWithRecommendedPreset(false, '[LocalAIStep]').catch((err: unknown) => {
      console.warn('[LocalAIStep] Local AI bootstrap failed:', err);
      onDownloadError?.('Local AI setup encountered an issue');
    });

    // Advance to next step immediately
    onNext({ consentGiven: true, downloadStarted: true });
  }, [onNext, onDownloadError]);

  return (
    <div className="rounded-2xl border border-stone-200 bg-white p-8 shadow-soft animate-fade-up">
      <div className="flex flex-col items-center mb-5">
        <img src="/ollama.svg" alt="Ollama" className="w-16 h-16 mb-3" />
        <h1 className="text-xl font-bold mb-2 text-stone-900">Run AI Models Locally with Ollama</h1>
        <p className="text-stone-600 text-sm text-center">
          OpenHuman will auto-install Ollama for you so that you can download and run AI models
          locally on your device.
        </p>
      </div>

      <div className="space-y-2 mb-5">
        <div className="rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
          <p className="text-xs text-stone-700">
            <span className="font-semibold">Complete Privacy</span>
            <span className="text-stone-600">
              &nbsp;- all data stays on your device. Nothing is sent to any third party or cloud.
            </span>
          </p>
        </div>
        <div className="rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
          <p className="text-xs text-stone-700">
            <span className="font-semibold">Absolutely Free</span>
            <span className="text-stone-600">
              &nbsp;- Ollama and the AI models are open-source. No subscription needed.
            </span>
          </p>
        </div>
        <div className="rounded-xl border border-amber-200 bg-amber-50 px-3 py-2">
          <p className="text-xs text-stone-700">
            <span className="font-semibold">Resource impact</span>
            <span className="text-stone-600">
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
