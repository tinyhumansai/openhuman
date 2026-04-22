import { useState } from 'react';

import { useCoreState } from '../../providers/CoreStateProvider';
import { userApi } from '../../services/api/userApi';
import { getDefaultEnabledTools } from '../../utils/toolDefinitions';
import BetaBanner from './components/BetaBanner';
import ContextGatheringStep from './steps/ContextGatheringStep';
import SkillsStep from './steps/SkillsStep';
import WelcomeStep from './steps/WelcomeStep';

interface OnboardingProps {
  onComplete?: () => void;
  onDefer?: () => void;
}

interface OnboardingDraft {
  connectedSources: string[];
}

const Onboarding = ({ onComplete, onDefer }: OnboardingProps) => {
  const { setOnboardingCompletedFlag, setOnboardingTasks, snapshot } = useCoreState();
  const [currentStep, setCurrentStep] = useState(0);
  const [draft, setDraft] = useState<OnboardingDraft>({ connectedSources: [] });

  const handleWelcomeNext = () => {
    setCurrentStep(1);
  };

  const handleNext = () => {
    if (currentStep < 2) {
      setCurrentStep(currentStep + 1);
    }
  };

  const handleBack = () => {
    if (currentStep > 0) {
      setCurrentStep(currentStep - 1);
    }
  };

  const handleSkillsNext = async (connectedSources: string[]) => {
    console.debug('[onboarding:handleSkillsNext]', { connectedSources });
    setDraft(prev => ({ ...prev, connectedSources }));
    if (connectedSources.length === 0) {
      // No sources connected — skip context gathering and finish onboarding.
      await handleContextNext(connectedSources);
    } else {
      handleNext();
    }
  };

  const handleContextNext = async (connectedSourcesOverride?: string[]) => {
    const sources = connectedSourcesOverride ?? draft.connectedSources;
    console.debug('[onboarding:handleContextNext]', { connectedSources: sources });
    await setOnboardingTasks({
      accessibilityPermissionGranted:
        snapshot.localState.onboardingTasks?.accessibilityPermissionGranted ?? false,
      localModelConsentGiven: false,
      localModelDownloadStarted: false,
      enabledTools: getDefaultEnabledTools(),
      connectedSources: sources,
      updatedAtMs: Date.now(),
    });

    // Notify backend (best-effort — don't block onboarding completion)
    console.debug('[onboarding:handleContextNext] notifying backend');
    try {
      await userApi.onboardingComplete();
    } catch {
      console.warn('[onboarding] Failed to notify backend of onboarding completion');
    }

    // Write onboarding_completed to core config (source of truth).
    // This is the authoritative flag — if it fails, don't complete.
    console.debug('[onboarding:handleContextNext] setting onboarding completed flag');
    try {
      await setOnboardingCompletedFlag(true);
    } catch (e) {
      console.error('[onboarding] Failed to persist onboarding_completed to core config', e);
      throw e;
    }

    onComplete?.();
  };

  const renderStep = () => {
    switch (currentStep) {
      case 0:
        return <WelcomeStep onNext={handleWelcomeNext} />;
      case 1:
        return <SkillsStep onNext={handleSkillsNext} onBack={handleBack} />;
      case 2:
        return (
          <ContextGatheringStep
            connectedSources={draft.connectedSources}
            onNext={handleContextNext}
            onBack={handleBack}
          />
        );
      default:
        return null;
    }
  };

  return (
    <div className="min-h-full relative flex items-center justify-center">
      {onDefer && (
        <div className="fixed top-4 right-0 z-20 sm:top-6 sm:right-6">
          <button
            type="button"
            onClick={onDefer}
            className="text-sm text-stone-600 hover:text-stone-900 transition-colors">
            Skip
          </button>
        </div>
      )}
      <div className="relative z-10 max-w-lg w-full mx-4">
        <BetaBanner />
        {renderStep()}
      </div>
    </div>
  );
};

export default Onboarding;
