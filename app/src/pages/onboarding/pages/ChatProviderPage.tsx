import { useState } from 'react';

import { useOnboardingContext } from '../OnboardingContext';
import OnboardingNextButton from '../components/OnboardingNextButton';

/**
 * Final onboarding step: pick a single chat provider.
 *
 * TODO: replace this stub with the real provider picker (WhatsApp /
 * Telegram / Slack / iMessage / …). For now it just lets the user
 * complete onboarding with no provider selected so the routed-pages
 * scaffolding can ship on its own.
 */
const ChatProviderPage = () => {
  const { completeAndExit } = useOnboardingContext();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleFinish = async () => {
    setError(null);
    setLoading(true);
    try {
      await completeAndExit();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Could not finish onboarding.');
      setLoading(false);
    }
  };

  return (
    <div
      data-testid="onboarding-chat-provider-step"
      className="rounded-2xl border border-stone-200 bg-white p-8 shadow-soft animate-fade-up">
      <div className="text-center mb-5">
        <h1 className="text-xl font-bold mb-2 text-stone-900">Pick your chat provider</h1>
        <p className="text-stone-500 text-sm leading-relaxed max-w-sm mx-auto">
          Choose one chat provider to start with. You can connect more later from Skills.
        </p>
      </div>

      <div className="rounded-xl border border-dashed border-stone-200 bg-stone-50 p-6 mb-5 text-center text-sm text-stone-500">
        Provider picker coming soon.
      </div>

      {error && <p className="text-coral-400 text-sm mb-3 text-center">{error}</p>}

      <OnboardingNextButton
        onClick={handleFinish}
        loading={loading}
        loadingLabel="Finishing…"
        label="Finish"
      />
    </div>
  );
};

export default ChatProviderPage;
