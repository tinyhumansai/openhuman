import { createContext, useCallback, useContext, useMemo } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { WalkthroughPhase, WalkthroughState } from '../../pages/onboarding/OnboardingContext';

export interface WalkthroughUIContextValue {
  /** Current walkthrough state from the onboarding draft. */
  state: WalkthroughState;
  /** Advance to the next phase. */
  advance: () => void;
  /** Skip the remaining walkthrough. */
  skip: () => void;
  /** Phase labels for the progress bar. */
  phaseLabels: Record<WalkthroughPhase, string>;
  /** Phase icons (emoji) for visual flair. */
  phaseIcons: Record<WalkthroughPhase, string>;
  /** Human-readable descriptions for action-card step keys. */
  stepLabels: Record<string, string>;
  /** Step descriptions explaining what each card does. */
  stepDescriptions: Record<string, string>;
}

const WalkthroughUIContext = createContext<WalkthroughUIContextValue | null>(null);

export function useWalkthroughUI(): WalkthroughUIContextValue {
  const ctx = useContext(WalkthroughUIContext);
  if (!ctx) {
    throw new Error('useWalkthroughUI must be used within a WalkthroughProvider');
  }
  return ctx;
}

interface WalkthroughProviderProps {
  state: WalkthroughState;
  onAdvance: (stepKey?: string) => void;
  onSkip: () => void;
  children: React.ReactNode;
}

/**
 * Provides UI-level walkthrough context: labels, icons, and action dispatchers.
 * Separates presentation concerns from the core OnboardingContext state machine.
 */
export const WalkthroughProvider = ({
  state,
  onAdvance,
  onSkip,
  children,
}: WalkthroughProviderProps) => {
  const { t } = useT();

  const phaseLabels: Record<WalkthroughPhase, string> = useMemo(
    () => ({
      welcome: t('walkthrough.phase.welcome', 'Welcome'),
      connect: t('walkthrough.phase.connect', 'Connect'),
      automate: t('walkthrough.phase.automate', 'Automate'),
      review: t('walkthrough.phase.review', 'Review'),
      done: t('walkthrough.phase.done', 'Done'),
    }),
    [t]
  );

  const phaseIcons: Record<WalkthroughPhase, string> = useMemo(
    () => ({ welcome: '👋', connect: '🔗', automate: '⚡', review: '✅', done: '🎉' }),
    []
  );

  const stepLabels: Record<string, string> = useMemo(
    () => ({
      start: t('walkthrough.step.start', 'Start setup'),
      gmail: t('walkthrough.step.gmail', 'Gmail'),
      slack: t('walkthrough.step.slack', 'Slack'),
      whatsapp: t('walkthrough.step.whatsapp', 'WhatsApp'),
      telegram: t('walkthrough.step.telegram', 'Telegram'),
      discord: t('walkthrough.step.discord', 'Discord'),
      briefings: t('walkthrough.step.briefings', 'Daily Briefings'),
      notifications: t('walkthrough.step.notifications', 'Smart Notifications'),
      scheduling: t('walkthrough.step.scheduling', 'Auto Scheduling'),
      summaries: t('walkthrough.step.summaries', 'Meeting Summaries'),
    }),
    [t]
  );

  const stepDescriptions: Record<string, string> = useMemo(
    () => ({
      start: t('walkthrough.desc.start', 'Begin with the recommended setup path'),
      gmail: t('walkthrough.desc.gmail', 'Connect your email for smart replies and summaries'),
      slack: t('walkthrough.desc.slack', 'Let your assistant join your team conversations'),
      whatsapp: t('walkthrough.desc.whatsapp', 'Chat with your assistant on the go'),
      telegram: t('walkthrough.desc.telegram', 'Secure messaging with your AI'),
      discord: t('walkthrough.desc.discord', 'Bring your assistant to your server'),
      briefings: t('walkthrough.desc.briefings', 'Get a daily summary of what matters'),
      notifications: t('walkthrough.desc.notifications', 'Stay informed without the noise'),
      scheduling: t('walkthrough.desc.scheduling', 'Let AI handle your calendar'),
      summaries: t('walkthrough.desc.summaries', 'Never miss a meeting detail again'),
    }),
    [t]
  );

  const advance = useCallback(() => {
    onAdvance();
  }, [onAdvance]);

  const value: WalkthroughUIContextValue = useMemo(
    () => ({ state, advance, skip: onSkip, phaseLabels, phaseIcons, stepLabels, stepDescriptions }),
    [state, advance, onSkip, phaseLabels, phaseIcons, stepLabels, stepDescriptions]
  );

  return <WalkthroughUIContext.Provider value={value}>{children}</WalkthroughUIContext.Provider>;
};

export default WalkthroughProvider;
