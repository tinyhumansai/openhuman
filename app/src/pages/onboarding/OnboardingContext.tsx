import { createContext, useContext } from 'react';

export type AiMode = 'cloud' | 'custom';

export type CustomStepKey =
  | 'inference'
  | 'voice'
  | 'oauth'
  | 'search'
  | 'embeddings'
  | 'memory'
  | 'activity'
  | 'vault';
export type CustomStepChoice = 'default' | 'configure';

/**
 * Walkthrough phases map to the narrative arc shown in the hero flow:
 *   welcome → connect → automate → review → done
 *
 * Each phase can carry its own progress metadata so the UI can render
 * animated progress bars, completed checkmarks, and contextual CTAs.
 */
export type WalkthroughPhase = 'welcome' | 'connect' | 'automate' | 'review' | 'done';

export interface WalkthroughStepState {
  /** Unique key for this step (e.g. 'gmail', 'slack', 'whatsapp'). */
  key: string;
  /** Whether the user has completed this action card. */
  completed: boolean;
  /** Optional metadata (e.g. connection status, config values). */
  meta?: Record<string, unknown>;
}

export interface WalkthroughState {
  /** Current phase in the narrative arc. */
  phase: WalkthroughPhase;
  /** Ordered list of action-card steps for the current phase. */
  steps: WalkthroughStepState[];
  /** Whether the walkthrough has been fully completed. */
  completed: boolean;
  /** Whether the walkthrough was skipped by the user. */
  skipped: boolean;
}

export interface OnboardingDraft {
  connectedSources: string[];
  /** Which AI provisioning path the user chose on the runtime-choice step. */
  aiMode?: AiMode;
  /** Per-domain choices made while walking the Custom wizard. */
  customChoices?: Partial<Record<CustomStepKey, CustomStepChoice>>;
  /** Walkthrough narrative state, persisted across onboarding steps. */
  walkthrough?: WalkthroughState;
}

export interface OnboardingContextValue {
  draft: OnboardingDraft;
  setDraft: (updater: (prev: OnboardingDraft) => OnboardingDraft) => void;
  /**
   * Persist `onboarding_completed=true`, notify the backend (best-effort), and
   * navigate to `/home`. Called by the final step.
   */
  completeAndExit: () => Promise<void>;
  /**
   * Advance the walkthrough to the next phase or mark a step as completed.
   */
  advanceWalkthrough: (stepKey?: string) => void;
  /**
   * Skip the remaining walkthrough and jump to the review phase.
   */
  skipWalkthrough: () => void;
}

export const OnboardingContext = createContext<OnboardingContextValue | null>(null);

export function useOnboardingContext(): OnboardingContextValue {
  const ctx = useContext(OnboardingContext);
  if (!ctx) {
    throw new Error('useOnboardingContext must be used within an OnboardingLayout');
  }
  return ctx;
}
