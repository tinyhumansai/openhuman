import { type ReactNode, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import OnboardingNextButton from '../components/OnboardingNextButton';
import WizardStepper from '../components/WizardStepper';
import type { CustomStepChoice } from '../OnboardingContext';

interface ChoiceCardProps {
  selected: boolean;
  onClick: () => void;
  accent: 'sage' | 'primary';
  title: string;
  description: string;
  testId: string;
  disabled?: boolean;
}

const ChoiceCard = ({
  selected,
  onClick,
  accent,
  title,
  description,
  testId,
  disabled = false,
}: ChoiceCardProps) => {
  const selectedClasses =
    accent === 'sage'
      ? '!border-sage-500 bg-sage-50 dark:bg-sage-500/10 shadow-sm'
      : '!border-primary-500 bg-primary-50 dark:bg-primary-500/15 shadow-sm';
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-pressed={selected}
      data-testid={testId}
      className={`flex h-full w-full flex-col rounded-2xl border-2 p-5 text-left transition-colors focus:outline-none disabled:cursor-not-allowed disabled:opacity-60 ${
        selected
          ? selectedClasses
          : '!border-stone-200 dark:!border-neutral-700 bg-white dark:bg-neutral-900 hover:!border-stone-300 dark:hover:!border-neutral-600 hover:bg-stone-50 dark:hover:bg-neutral-800/60'
      }`}>
      <h3 className="text-base font-semibold text-stone-900 dark:text-neutral-100">{title}</h3>
      <p className="mt-1 text-xs text-stone-600 dark:text-neutral-300 leading-relaxed">
        {description}
      </p>
    </button>
  );
};

interface CustomWizardStepProps {
  stepIndex: number;
  stepCount: number;
  title: string;
  subtitle: string;
  defaultDescription: string;
  configureDescription: string;
  /** Inline content rendered below the choice cards when 'configure' is picked. */
  configureContent?: ReactNode;
  choice: CustomStepChoice | null;
  onChoiceChange: (choice: CustomStepChoice) => void;
  onBack: () => void;
  onContinue: () => void | Promise<void>;
  /** Continue label override (used for the final "Finish setup" step). */
  continueLabel?: string;
  /** Disable the continue button (e.g. while an inline save is in flight). */
  continueDisabled?: boolean;
  /** Replace the continue button text with a busy label while loading. */
  continueLoading?: boolean;
  continueLoadingLabel?: string;
  testId?: string;
  defaultDisabled?: boolean;
  defaultDisabledReason?: string;
  hideChoiceCards?: boolean;
}

const CustomWizardStep = ({
  stepIndex,
  stepCount,
  title,
  subtitle,
  defaultDescription,
  configureDescription,
  configureContent,
  choice,
  onChoiceChange,
  onBack,
  onContinue,
  continueLabel,
  continueDisabled,
  continueLoading,
  continueLoadingLabel,
  testId,
  defaultDisabled = false,
  defaultDisabledReason,
  hideChoiceCards = false,
}: CustomWizardStepProps) => {
  const { t } = useT();
  const [isContinuing, setIsContinuing] = useState(false);

  const handleContinue = async () => {
    if (isContinuing || choice === null || continueDisabled) return;
    try {
      setIsContinuing(true);
      await onContinue();
    } finally {
      setIsContinuing(false);
    }
  };

  const stepperLabels = [
    t('onboarding.custom.stepperInference'),
    t('onboarding.custom.stepperVoice'),
    t('onboarding.custom.stepperOAuth'),
    t('onboarding.custom.stepperSearch'),
    t('onboarding.custom.stepperEmbeddings'),
    t('onboarding.custom.stepperMemory'),
  ].slice(0, stepCount);

  return (
    <div
      data-testid={testId ?? 'onboarding-custom-wizard-step'}
      className="rounded-2xl bg-white dark:bg-neutral-900 p-10 shadow-soft animate-fade-up">
      <WizardStepper labels={stepperLabels} activeIndex={stepIndex} />

      <h1 className="mt-8 text-2xl font-display text-stone-900 dark:text-neutral-100 leading-tight">
        {title}
      </h1>
      <p className="mt-2 text-sm text-stone-500 dark:text-neutral-400 leading-relaxed">
        {subtitle}
      </p>

      {!hideChoiceCards ? (
        <>
          <div className="mt-6 grid grid-cols-1 gap-3 sm:grid-cols-2 sm:items-stretch">
            <ChoiceCard
              testId={`${testId ?? 'onboarding-custom-wizard-step'}-default`}
              accent="sage"
              selected={choice === 'default'}
              onClick={() => onChoiceChange('default')}
              disabled={defaultDisabled}
              title={t('onboarding.custom.defaultTitle')}
              description={defaultDescription || t('onboarding.custom.defaultSubtitle')}
            />
            <ChoiceCard
              testId={`${testId ?? 'onboarding-custom-wizard-step'}-configure`}
              accent="primary"
              selected={choice === 'configure'}
              onClick={() => onChoiceChange('configure')}
              title={t('onboarding.custom.configureTitle')}
              description={configureDescription || t('onboarding.custom.configureSubtitle')}
            />
          </div>

          {defaultDisabled && defaultDisabledReason ? (
            <p className="mt-3 text-xs text-stone-500 dark:text-neutral-400 leading-relaxed">
              {defaultDisabledReason}
            </p>
          ) : null}
        </>
      ) : null}

      {(choice === 'configure' || hideChoiceCards) && configureContent ? (
        <div className="mt-6 rounded-2xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-5">
          {configureContent}
        </div>
      ) : null}

      <div className="mt-8 flex items-center gap-3">
        <button
          type="button"
          onClick={onBack}
          className="rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-4 py-2.5 text-sm font-medium text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800/60 focus:outline-none">
          {t('onboarding.custom.back')}
        </button>
        <div className="flex-1">
          <OnboardingNextButton
            label={continueLabel ?? t('onboarding.custom.continue')}
            onClick={() => void handleContinue()}
            disabled={choice === null || continueDisabled || isContinuing}
            loading={continueLoading || isContinuing}
            loadingLabel={continueLoadingLabel}
          />
        </div>
      </div>
    </div>
  );
};

export default CustomWizardStep;
