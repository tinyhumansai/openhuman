import { useState } from 'react';

import { ProviderIcon } from '../../../components/accounts/providerIcons';
import WebviewLoginModal from '../components/WebviewLoginModal';
import OnboardingNextButton from '../components/OnboardingNextButton';

interface SkillsStepProps {
  onNext: (connectedSources: string[]) => void | Promise<void>;
  onBack?: () => void;
}

/**
 * Onboarding "Connect your tools" step. Replaces the previous Composio
 * OAuth list with the in-app webview-login flow. For the first cut we
 * support Gmail only; more providers will follow the same pattern
 * (open a webview, let the user sign in, mark connected).
 */
const SkillsStep = ({ onNext, onBack: _onBack }: SkillsStepProps) => {
  const [loginOpen, setLoginOpen] = useState(false);
  const [gmailConnected, setGmailConnected] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleConnected = () => {
    console.debug('[onboarding:skills] gmail connected via webview');
    setGmailConnected(true);
    setLoginOpen(false);
  };

  const handleContinue = async () => {
    setError(null);
    setSubmitting(true);
    try {
      const sources = gmailConnected ? ['webview:gmail'] : [];
      await onNext(sources);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Something went wrong. Please try again.');
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="rounded-2xl border border-stone-200 bg-white p-8 shadow-soft animate-fade-up">
      <div className="text-center mb-4">
        <h1 className="text-xl font-bold mb-2 text-stone-900">Connect your tools</h1>
        <p className="text-stone-600 text-sm">
          Sign in to the apps you already use so OpenHuman can build context for your agent.
          You'll log in inside an embedded browser — your password never touches OpenHuman's
          servers.
        </p>
      </div>

      <div className="mb-4 space-y-2">
        <button
          type="button"
          onClick={() => setLoginOpen(true)}
          data-testid="onboarding-skills-gmail-button"
          className="w-full flex items-center gap-3 rounded-xl border border-stone-100 bg-white p-3 transition-colors hover:bg-stone-50 text-left">
          <div className="flex h-8 w-8 flex-shrink-0 items-center justify-center">
            <ProviderIcon provider="gmail" className="h-6 w-6" />
          </div>

          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="truncate text-sm font-semibold text-stone-900">Gmail</span>
              {gmailConnected && (
                <>
                  <div className="h-1.5 w-1.5 flex-shrink-0 rounded-full bg-sage-500" />
                  <span className="flex-shrink-0 text-xs text-sage-600">Connected</span>
                </>
              )}
            </div>
            <p className="mt-0.5 line-clamp-1 text-xs leading-relaxed text-stone-500">
              Sign in to Gmail in an embedded browser. Used to find context about you.
            </p>
          </div>

          <span
            className={`flex-shrink-0 rounded-lg border px-3 py-1.5 text-[11px] font-medium transition-colors ${
              gmailConnected
                ? 'border-sage-200 bg-sage-50 text-sage-700'
                : 'border-primary-200 bg-primary-50 text-primary-700'
            }`}>
            {gmailConnected ? 'Manage' : 'Connect'}
          </span>
        </button>

        <div className="rounded-xl border border-stone-100 bg-stone-50 px-3 py-2.5 text-center">
          <p className="text-xs text-stone-400">
            More providers (Slack, Notion, GitHub, …) available after setup
          </p>
        </div>
      </div>

      {error && <p className="text-coral-400 text-sm mb-3 text-center">{error}</p>}

      <OnboardingNextButton
        onClick={handleContinue}
        loading={submitting}
        loadingLabel="Loading..."
        label={gmailConnected ? 'Continue' : 'Skip for Now'}
      />

      {loginOpen && (
        <WebviewLoginModal
          provider="gmail"
          label="Gmail"
          onClose={() => setLoginOpen(false)}
          onConnected={handleConnected}
        />
      )}
    </div>
  );
};

export default SkillsStep;
