import { useEffect, useMemo, useState } from 'react';

import { useCoreState } from '../../providers/CoreStateProvider';
import { referralApi } from '../../services/api/referralApi';
import { userApi } from '../../services/api/userApi';
import { getDefaultEnabledTools } from '../../utils/toolDefinitions';
import ReferralApplyStep from './steps/ReferralApplyStep';
import ScreenPermissionsStep from './steps/ScreenPermissionsStep';
import SkillsStep from './steps/SkillsStep';
import WelcomeStep from './steps/WelcomeStep';

interface OnboardingProps {
  onComplete?: () => void;
  onDefer?: () => void;
}

interface OnboardingDraft {
  accessibilityPermissionGranted: boolean;
  connectedSources: string[];
}

function hasReferralFromProfile(
  user:
    | { referral?: { invitedBy?: string | null; invitedByCode?: string | null } }
    | null
    | undefined
): boolean {
  return !!(user?.referral?.invitedBy || user?.referral?.invitedByCode);
}

/** When referral is skipped, step index 1 (apply) is not shown — treat as screen permissions (2). */
function resolveOnboardingStep(currentStep: number, skipReferralStep: boolean): number {
  if (skipReferralStep && currentStep === 1) {
    return 2;
  }
  return currentStep;
}

const Onboarding = ({ onComplete, onDefer }: OnboardingProps) => {
  const { setOnboardingCompletedFlag, setOnboardingTasks, snapshot } = useCoreState();
  const [currentStep, setCurrentStep] = useState(0);
  const [draft, setDraft] = useState<OnboardingDraft>({
    accessibilityPermissionGranted: false,
    connectedSources: [],
  });
  /** Last session token for which referral stats prefetch finished (async path only). */
  const [referralStatsToken, setReferralStatsToken] = useState<string | null>(null);
  const [skipReferralFromStats, setSkipReferralFromStats] = useState(false);
  const [referralAppliedThisSession, setReferralAppliedThisSession] = useState(false);

  const token = snapshot.sessionToken;
  const currentUser = snapshot.currentUser;

  const profileAlreadyReferred = useMemo(() => hasReferralFromProfile(currentUser), [currentUser]);
  const needsReferralStatsPrefetch = !!(token && !profileAlreadyReferred);

  useEffect(() => {
    if (!needsReferralStatsPrefetch) {
      return;
    }

    let cancelled = false;
    (async () => {
      try {
        const stats = await referralApi.getStats();
        const applied =
          typeof stats.appliedReferralCode === 'string' && stats.appliedReferralCode.trim() !== '';
        if (!cancelled) {
          setSkipReferralFromStats(applied);
          setReferralStatsToken(token);
        }
      } catch {
        console.debug('[onboarding] referral preflight failed; showing referral step');
        if (!cancelled) {
          setSkipReferralFromStats(false);
          setReferralStatsToken(token);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [needsReferralStatsPrefetch, token, profileAlreadyReferred]);

  const referralGateReady = !token || profileAlreadyReferred || referralStatsToken === token;

  const skipReferralStep = !token
    ? false
    : profileAlreadyReferred
      ? true
      : referralStatsToken === token && skipReferralFromStats;

  const resolvedStep = resolveOnboardingStep(currentStep, skipReferralStep);

  const handleWelcomeNext = () => {
    if (skipReferralStep) {
      setCurrentStep(2);
    } else {
      setCurrentStep(1);
    }
  };

  const handleNext = () => {
    const logical = resolveOnboardingStep(currentStep, skipReferralStep);
    if (logical < 3) {
      setCurrentStep(logical + 1);
    }
  };

  const handleBack = () => {
    const logical = resolveOnboardingStep(currentStep, skipReferralStep);
    if (logical <= 0) return;
    if (
      logical === 2 &&
      (skipReferralStep || profileAlreadyReferred || referralAppliedThisSession)
    ) {
      setCurrentStep(0);
      return;
    }
    setCurrentStep(logical - 1);
  };

  const handleAccessibilityNext = (accessibilityPermissionGranted: boolean) => {
    setDraft(prev => ({ ...prev, accessibilityPermissionGranted }));
    handleNext();
  };

  const handleSkillsNext = async (connectedSources: string[]) => {
    setDraft(prev => ({ ...prev, connectedSources }));

    await setOnboardingTasks({
      accessibilityPermissionGranted: draft.accessibilityPermissionGranted,
      localModelConsentGiven: false,
      localModelDownloadStarted: false,
      enabledTools: getDefaultEnabledTools(),
      connectedSources,
      updatedAtMs: Date.now(),
    });

    // Notify backend (best-effort — don't block onboarding completion)
    try {
      await userApi.onboardingComplete();
    } catch {
      console.warn('[onboarding] Failed to notify backend of onboarding completion');
    }

    // Write onboarding_completed to core config (source of truth)
    try {
      await setOnboardingCompletedFlag(true);
    } catch {
      console.warn('[onboarding] Failed to persist onboarding_completed to core config');
    }

    onComplete?.();
  };

  const renderStep = () => {
    switch (resolvedStep) {
      case 0:
        return (
          <WelcomeStep
            onNext={handleWelcomeNext}
            nextDisabled={!referralGateReady}
            nextLoading={!!token && !referralGateReady}
            nextLoadingLabel="Checking account…"
          />
        );
      case 1:
        return (
          <ReferralApplyStep
            onNext={handleNext}
            onBack={handleBack}
            onApplied={() => setReferralAppliedThisSession(true)}
          />
        );
      case 2:
        return <ScreenPermissionsStep onNext={handleAccessibilityNext} onBack={handleBack} />;
      case 3:
        return <SkillsStep onNext={handleSkillsNext} onBack={handleBack} />;
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
        {renderStep()}
      </div>
    </div>
  );
};

export default Onboarding;
