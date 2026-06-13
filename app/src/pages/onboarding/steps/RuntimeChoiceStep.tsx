import { useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import OnboardingNextButton from '../components/OnboardingNextButton';
import type { AiMode } from '../OnboardingContext';

interface RuntimeChoiceStepProps {
  onNext: (mode: AiMode) => void;
}

type Accent = 'sage' | 'primary';

interface ChoiceCardProps {
  selected: boolean;
  accent: Accent;
  onClick: () => void;
  badge?: string;
  title: string;
  tagline: string;
  features: string[];
  highlight?: string;
  testId: string;
}

const ACCENT_CLASSES: Record<
  Accent,
  { selected: string; dot: string; badge: string; highlight: string }
> = {
  sage: {
    selected: '!border-sage-500 bg-sage-50 dark:bg-sage-500/10 shadow-sm',
    dot: 'bg-sage-500',
    badge: 'bg-sage-500/10 text-sage-700 dark:text-sage-300',
    highlight:
      'border-sage-300 dark:border-sage-500/40 bg-sage-100 dark:bg-sage-500/20 text-sage-800 dark:text-sage-200',
  },
  primary: {
    selected: '!border-primary-500 bg-primary-50 dark:bg-primary-500/15 shadow-sm',
    dot: 'bg-primary-500',
    badge: 'bg-primary-500/10 text-primary-600 dark:text-primary-300',
    highlight:
      'border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/15 text-primary-700 dark:text-primary-300',
  },
};

const ChoiceCard = ({
  selected,
  accent,
  onClick,
  badge,
  title,
  tagline,
  features,
  highlight,
  testId,
}: ChoiceCardProps) => {
  const accentClasses = ACCENT_CLASSES[accent];
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={selected}
      data-testid={testId}
      className={`flex h-full w-full flex-col rounded-2xl border-2 p-5 text-left transition-colors focus:outline-none ${
        selected
          ? accentClasses.selected
          : '!border-stone-200 dark:!border-neutral-700 bg-white dark:bg-neutral-900 hover:!border-stone-300 dark:hover:!border-neutral-600 hover:bg-stone-50 dark:hover:bg-neutral-800/60'
      }`}>
      <div className="flex items-start justify-between gap-3">
        <h3 className="text-base font-semibold text-stone-900 dark:text-neutral-100">{title}</h3>
        {badge ? (
          <span
            className={`rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide ${accentClasses.badge}`}>
            {badge}
          </span>
        ) : null}
      </div>
      <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">{tagline}</p>
      <ul className="mt-3 flex flex-1 flex-col gap-1.5">
        {features.map(f => (
          <li
            key={f}
            className="flex items-start gap-2 text-xs text-stone-700 dark:text-neutral-200">
            <span
              aria-hidden
              className={`mt-1 inline-block h-1.5 w-1.5 flex-none rounded-full ${accentClasses.dot}`}
            />
            <span>{f}</span>
          </li>
        ))}
      </ul>
      {highlight ? (
        <div
          className={`mt-4 rounded-lg border px-3 py-2 text-xs font-semibold ${accentClasses.highlight}`}>
          {highlight}
        </div>
      ) : null}
    </button>
  );
};

const RuntimeChoiceStep = ({ onNext }: RuntimeChoiceStepProps) => {
  const { t } = useT();
  const [selected, setSelected] = useState<AiMode | null>('cloud');

  const cloudFeatures = [
    t('onboarding.runtimeChoice.cloud.f1'),
    t('onboarding.runtimeChoice.cloud.f2'),
    t('onboarding.runtimeChoice.cloud.f3'),
    t('onboarding.runtimeChoice.cloud.f4'),
    t('onboarding.runtimeChoice.cloud.f5'),
  ];
  const customFeatures = [
    t('onboarding.runtimeChoice.custom.f1'),
    t('onboarding.runtimeChoice.custom.f2'),
    t('onboarding.runtimeChoice.custom.f3'),
    t('onboarding.runtimeChoice.custom.f4'),
    t('onboarding.runtimeChoice.custom.f5'),
  ];

  return (
    <div
      data-testid="onboarding-runtime-choice-step"
      className="rounded-2xl bg-white dark:bg-neutral-900 p-10 shadow-soft animate-fade-up">
      <div className="text-center">
        <h1 className="text-2xl font-display text-stone-900 dark:text-neutral-100 mb-2 leading-tight">
          {t('onboarding.runtimeChoice.title')}
        </h1>
        <p className="text-stone-500 dark:text-neutral-400 text-sm leading-relaxed">
          {t('onboarding.runtimeChoice.subtitle')}
        </p>
      </div>

      <div className="mt-6 grid grid-cols-1 gap-3 sm:grid-cols-2 sm:items-stretch">
        <ChoiceCard
          testId="onboarding-runtime-choice-cloud"
          accent="sage"
          selected={selected === 'cloud'}
          onClick={() => setSelected('cloud')}
          badge={t('onboarding.runtimeChoice.recommended')}
          title={t('onboarding.runtimeChoice.cloud.title')}
          tagline={t('onboarding.runtimeChoice.cloud.tagline')}
          features={cloudFeatures}
          highlight={t('onboarding.runtimeChoice.cloud.creditHighlight')}
        />
        <ChoiceCard
          testId="onboarding-runtime-choice-custom"
          accent="primary"
          selected={selected === 'custom'}
          onClick={() => setSelected('custom')}
          title={t('onboarding.runtimeChoice.custom.title')}
          tagline={t('onboarding.runtimeChoice.custom.tagline')}
          features={customFeatures}
        />
      </div>

      <div className="mt-8">
        <OnboardingNextButton
          label={
            selected === 'custom'
              ? t('onboarding.runtimeChoice.continueCustom')
              : t('onboarding.runtimeChoice.continueCloud')
          }
          disabled={selected === null}
          onClick={() => selected && onNext(selected)}
        />
      </div>
    </div>
  );
};

export default RuntimeChoiceStep;
