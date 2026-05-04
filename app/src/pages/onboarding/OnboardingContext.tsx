import { createContext, useContext } from 'react';

export interface OnboardingDraft {
  connectedSources: string[];
}

export interface OnboardingContextValue {
  draft: OnboardingDraft;
  setDraft: (updater: (prev: OnboardingDraft) => OnboardingDraft) => void;
  /**
   * Persist `onboarding_completed=true`, notify the backend (best-effort), and
   * navigate to `/home`. Called by the final step.
   */
  completeAndExit: () => Promise<void>;
}

export const OnboardingContext = createContext<OnboardingContextValue | null>(null);

export function useOnboardingContext(): OnboardingContextValue {
  const ctx = useContext(OnboardingContext);
  if (!ctx) {
    throw new Error('useOnboardingContext must be used within an OnboardingLayout');
  }
  return ctx;
}
