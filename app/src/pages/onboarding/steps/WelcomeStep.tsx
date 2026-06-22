import WhatLeavesLink from '../../../features/privacy/WhatLeavesLink';
import { useT } from '../../../lib/i18n/I18nContext';
import OnboardingNextButton from '../components/OnboardingNextButton';
import WelcomeHeroCard from '../components/WelcomeHeroCard';

interface WelcomeStepProps {
  onNext: () => void;
}

/** Arrow connector between flow steps */
const FlowArrow = () => (
  <div
    className="hidden sm:flex items-center justify-center text-neutral-300 dark:text-neutral-600 shrink-0"
    aria-hidden="true">
    <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M13 7l5 5m0 0l-5 5m5-5H6"
      />
    </svg>
  </div>
);

const WelcomeStep = ({ onNext }: WelcomeStepProps) => {
  const { t } = useT();

  return (
    <div
      data-testid="onboarding-welcome-step"
      className="rounded-2xl bg-white dark:bg-neutral-900 px-6 py-8 sm:px-10 sm:py-10 shadow-soft animate-fade-up">
      {/* ── Hero section ── */}
      <div className="flex flex-col items-center text-center mb-10">
        <img src="/logo.png" alt={t('welcome.logoAlt')} className="w-16 h-16 rounded-2xl mb-5" />
        <h1 className="text-3xl sm:text-4xl font-display text-stone-900 dark:text-neutral-100 mb-3 leading-tight tracking-tight">
          {t('onboarding.welcome.heroTitle')}
        </h1>
        <p className="text-sm text-stone-500 dark:text-neutral-400 leading-relaxed max-w-lg">
          {t('onboarding.welcome.heroSubtitle')}
        </p>
      </div>

      {/* ── Narrative flow: Connect → Automate → Deliver ── */}
      <div className="flex flex-col sm:flex-row items-stretch sm:items-start gap-3 sm:gap-0 mb-8">
        {/* Connect */}
        <div className="flex-1 flex flex-col items-center text-center gap-2 px-4 py-5 rounded-xl bg-primary-50/60 dark:bg-primary-500/5 border border-primary-100 dark:border-primary-500/10">
          <div className="flex items-center justify-center w-10 h-10 rounded-full bg-white dark:bg-neutral-800 border border-primary-200 dark:border-primary-500/20 text-primary-600 dark:text-primary-400 mb-1">
            <svg
              className="w-5 h-5"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
              aria-hidden="true">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1"
              />
            </svg>
          </div>
          <span className="text-sm font-semibold text-primary-700 dark:text-primary-300">
            {t('onboarding.welcome.inputTitle')}
          </span>
          <p className="text-xs text-stone-500 dark:text-neutral-400 leading-relaxed max-w-[180px]">
            {t('onboarding.welcome.inputDesc')}
          </p>
        </div>

        <FlowArrow />

        {/* Automate (hub) */}
        <div className="flex-1 flex flex-col items-center text-center gap-2 px-4 py-5 rounded-xl bg-amber-50/60 dark:bg-amber-500/5 border border-amber-100 dark:border-amber-500/10">
          <div className="flex items-center justify-center w-10 h-10 rounded-full bg-white dark:bg-neutral-800 border border-amber-200 dark:border-amber-500/20 text-amber-600 dark:text-amber-400 mb-1">
            <svg
              className="w-5 h-5"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
              aria-hidden="true">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"
              />
            </svg>
          </div>
          <span className="text-sm font-semibold text-amber-700 dark:text-amber-300">
            {t('onboarding.welcome.hubTitle')}
          </span>
          <p className="text-xs text-stone-500 dark:text-neutral-400 leading-relaxed max-w-[180px]">
            {t('onboarding.welcome.hubDesc')}
          </p>
        </div>

        <FlowArrow />

        {/* Deliver */}
        <div className="flex-1 flex flex-col items-center text-center gap-2 px-4 py-5 rounded-xl bg-emerald-50/60 dark:bg-emerald-500/5 border border-emerald-100 dark:border-emerald-500/10">
          <div className="flex items-center justify-center w-10 h-10 rounded-full bg-white dark:bg-neutral-800 border border-emerald-200 dark:border-emerald-500/20 text-emerald-600 dark:text-emerald-400 mb-1">
            <svg
              className="w-5 h-5"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
              aria-hidden="true">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"
              />
            </svg>
          </div>
          <span className="text-sm font-semibold text-emerald-700 dark:text-emerald-300">
            {t('onboarding.welcome.outputTitle')}
          </span>
          <p className="text-xs text-stone-500 dark:text-neutral-400 leading-relaxed max-w-[180px]">
            {t('onboarding.welcome.outputDesc')}
          </p>
        </div>
      </div>

      {/* ── Capability cards ── */}
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-3 mb-8">
        <WelcomeHeroCard
          icon={
            <svg
              className="w-5 h-5"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
              aria-hidden="true">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M9 17V7m0 10a2 2 0 01-2 2H5a2 2 0 01-2-2V7a2 2 0 012-2h2a2 2 0 012 2m0 10a2 2 0 002 2h2a2 2 0 002-2M9 7a2 2 0 012-2h2a2 2 0 012 2m0 10V7m0 10a2 2 0 002 2h2a2 2 0 002-2V7a2 2 0 00-2-2h-2a2 2 0 00-2 2"
              />
            </svg>
          }
          title={t('onboarding.welcome.capability1Title')}
          description={t('onboarding.welcome.capability1Desc')}
        />
        <WelcomeHeroCard
          icon={
            <svg
              className="w-5 h-5"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
              aria-hidden="true">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M13 10V3L4 14h7v7l9-11h-7z"
              />
            </svg>
          }
          title={t('onboarding.welcome.capability2Title')}
          description={t('onboarding.welcome.capability2Desc')}
        />
        <WelcomeHeroCard
          icon={
            <svg
              className="w-5 h-5"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
              aria-hidden="true">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z"
              />
            </svg>
          }
          title={t('onboarding.welcome.capability3Title')}
          description={t('onboarding.welcome.capability3Desc')}
        />
      </div>

      {/* ── CTA ── */}
      <div className="mb-4">
        <OnboardingNextButton label={t('onboarding.getStarted')} onClick={onNext} />
      </div>

      {/* ── Privacy link ── */}
      <div className="flex justify-center">
        <WhatLeavesLink />
      </div>
    </div>
  );
};

export default WelcomeStep;
