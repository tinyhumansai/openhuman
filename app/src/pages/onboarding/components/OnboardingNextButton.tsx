interface OnboardingNextButtonProps {
  label?: string;
  onClick: () => void;
  disabled?: boolean;
  loading?: boolean;
  loadingLabel?: string;
}

const OnboardingNextButton = ({
  label = 'Continue',
  onClick,
  disabled = false,
  loading = false,
  loadingLabel,
}: OnboardingNextButtonProps) => (
  <button
    type="button"
    data-testid="onboarding-next-button"
    onClick={onClick}
    disabled={disabled || loading}
    className="w-full py-2.5 bg-primary-500 hover:bg-primary-600 active:bg-primary-700 text-white text-sm font-medium rounded-xl transition-colors disabled:opacity-50 disabled:cursor-not-allowed">
    {loading ? (loadingLabel ?? label) : label}
  </button>
);

export default OnboardingNextButton;
