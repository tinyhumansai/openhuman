import { createContext, useContext } from 'react';

export type AiMode = 'cloud' | 'custom';

export type CustomStepKey = 'inference' | 'voice' | 'oauth' | 'search' | 'memory';
export type CustomStepChoice = 'default' | 'configure';

export interface OnboardingDraft {
  connectedSources: string[];
  /** Which AI provisioning path the user chose on the runtime-choice step. */
  aiMode?: AiMode;
  /** Per-domain choices made while walking the Custom wizard. */
  customChoices?: Partial<Record<CustomStepKey, CustomStepChoice>>;
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
