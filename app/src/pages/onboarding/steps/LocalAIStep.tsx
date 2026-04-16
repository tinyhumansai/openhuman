import { useCallback, useEffect, useRef, useState } from 'react';

import {
  bootstrapLocalAiWithRecommendedPreset,
  ensureRecommendedLocalAiPresetIfNeeded,
} from '../../../utils/localAiBootstrap';
import OnboardingNextButton from '../components/OnboardingNextButton';

/* ---------- component ---------- */

interface LocalAIStepProps {
  onNext: (result: { consentGiven: boolean; downloadStarted: boolean }) => void;
  onBack?: () => void;
  onDownloadError?: (message: string) => void;
}

const LocalAIStep = ({ onNext, onBack: _onBack, onDownloadError }: LocalAIStepProps) => {
  const downloadStartedRef = useRef(false);
  const [recommendDisabled, setRecommendDisabled] = useState<boolean | null>(null);

  useEffect(() => {
    let cancelled = false;
    ensureRecommendedLocalAiPresetIfNeeded('[LocalAIStep:probe]')
      .then(result => {
        if (!cancelled) {
          setRecommendDisabled(result.presets.recommend_disabled ?? false);
        }
      })
      .catch(() => {
        if (!cancelled) setRecommendDisabled(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

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

  const handleSkip = useCallback(() => {
    console.debug('[LocalAIStep] skipping local AI — using cloud fallback');
    onNext({ consentGiven: false, downloadStarted: false });
  }, [onNext]);

  // Still probing device — show nothing yet.
  if (recommendDisabled === null) {
    return null;
  }

  // Low-RAM device: show cloud fallback option as the primary path.
  if (recommendDisabled) {
    return (
      <div className="rounded-2xl border border-stone-200 bg-white p-8 shadow-soft animate-fade-up">
        <div className="flex flex-col items-center mb-5">
          <div className="flex h-16 w-16 items-center justify-center rounded-full bg-primary-50 mb-3">
            <svg
              className="h-8 w-8 text-primary-500"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              strokeWidth={1.5}>
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                d="M2.25 15a4.5 4.5 0 004.5 4.5H18a3.75 3.75 0 001.332-7.257 3 3 0 00-3.758-3.848 5.25 5.25 0 00-10.233 2.33A4.502 4.502 0 002.25 15z"
              />
            </svg>
          </div>
          <h1 className="text-xl font-bold mb-2 text-stone-900">AI — Cloud Mode</h1>
          <p className="text-stone-600 text-sm text-center">
            Your device has limited RAM, so we&apos;ll use a fast, lightweight cloud model for AI
            features. You can switch to local AI later in Settings.
          </p>
        </div>

        <div className="space-y-2 mb-5">
          <div className="rounded-xl border border-primary-200 bg-primary-50 px-3 py-2">
            <p className="text-xs text-stone-700">
              <span className="font-semibold">Fast &amp; lightweight</span>
              <span className="text-stone-600">
                &nbsp;— uses a cheap cloud summarizer model with minimal latency.
              </span>
            </p>
          </div>
          <div className="rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
            <p className="text-xs text-stone-700">
              <span className="font-semibold">No downloads needed</span>
              <span className="text-stone-600">
                &nbsp;— no large model files or Ollama install required.
              </span>
            </p>
          </div>
          <div className="rounded-xl border border-amber-200 bg-amber-50 px-3 py-2">
            <p className="text-xs text-stone-700">
              <span className="font-semibold">Requires internet</span>
              <span className="text-stone-600">
                &nbsp;— AI features need an active connection. You can opt into local AI in Settings
                if preferred.
              </span>
            </p>
          </div>
        </div>

        <OnboardingNextButton label="Continue with Cloud" onClick={handleSkip} />

        <button
          type="button"
          onClick={handleConsent}
          className="mt-3 w-full text-center text-xs text-stone-400 hover:text-stone-600 transition-colors">
          Use local AI anyway (not recommended for your device)
        </button>
      </div>
    );
  }

  // Sufficient RAM: show the standard local AI onboarding.
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

      <button
        type="button"
        onClick={handleSkip}
        className="mt-3 w-full text-center text-xs text-stone-400 hover:text-stone-600 transition-colors">
        Skip — use cloud AI instead
      </button>
    </div>
  );
};

export default LocalAIStep;
