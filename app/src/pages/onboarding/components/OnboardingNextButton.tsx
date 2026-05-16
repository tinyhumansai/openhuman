import { useT } from '../../../lib/i18n/I18nContext';

interface OnboardingNextButtonProps {
  label?: string;
  onClick: () => void;
  disabled?: boolean;
  loading?: boolean;
  loadingLabel?: string;
}

const OnboardingNextButton = ({
  label,
  onClick,
  disabled = false,
  loading = false,
  loadingLabel,
}: OnboardingNextButtonProps) => {
  const { t } = useT();
  const effectiveLabel = label ?? t('common.continue');
  const effectiveLoadingLabel = loadingLabel ?? effectiveLabel;
  return (
    <button
      type="button"
      data-testid="onboarding-next-button"
      onClick={onClick}
      disabled={disabled || loading}
      className="w-full py-2.5 bg-primary-500 hover:bg-primary-600 active:bg-primary-700 text-white text-sm font-medium rounded-xl transition-colors disabled:opacity-50 disabled:cursor-not-allowed">
      {loading ? effectiveLoadingLabel : effectiveLabel}
    </button>
  );
};

export default OnboardingNextButton;
