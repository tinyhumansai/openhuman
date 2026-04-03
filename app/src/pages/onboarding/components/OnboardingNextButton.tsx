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
    onClick={onClick}
    disabled={disabled || loading}
    className="w-full py-2.5 btn-primary text-sm font-medium rounded-xl border transition-colors border-stone-600 hover:border-sage-500 hover:bg-sage-500/10">
    {loading ? (loadingLabel ?? label) : label}
  </button>
);

export default OnboardingNextButton;
