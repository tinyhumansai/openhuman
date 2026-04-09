import { useState } from 'react';

import { useCoreState } from '../../../providers/CoreStateProvider';
import { referralApi } from '../../../services/api/referralApi';

interface ReferralApplyStepProps {
  onNext: () => void;
  onBack: () => void;
  /** Called after a successful apply so onboarding can skip showing this step when navigating back. */
  onApplied?: () => void;
}

/**
 * Optional step: attribute the signed-in user to a referrer via POST /referral/apply.
 */
const ReferralApplyStep = ({ onNext, onBack, onApplied }: ReferralApplyStepProps) => {
  const { refresh } = useCoreState();
  const [code, setCode] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState(false);

  const handleApply = async () => {
    const trimmed = code.trim();
    if (!trimmed) return;

    setIsLoading(true);
    setError(null);

    try {
      await referralApi.applyCode(trimmed);
      setSuccess(true);
      try {
        await refresh();
      } catch {
        console.warn('[onboarding] referral apply: refresh after apply failed');
      }
      onApplied?.();
      console.debug('[onboarding] referral code applied');
      setTimeout(() => onNext(), 1200);
    } catch (err) {
      const msg =
        err && typeof err === 'object' && 'error' in err
          ? String((err as { error: string }).error)
          : 'Could not apply referral code';
      setError(msg);
    } finally {
      setIsLoading(false);
    }
  };

  return (
    <div className="rounded-2xl border border-stone-200 bg-white p-8 shadow-soft animate-fade-up">
      <div className="text-center mb-6">
        <h1 className="text-xl font-bold mb-2 text-stone-900">Referral code</h1>
        <p className="text-stone-600 text-sm">
          If a friend shared OpenHuman with you, enter their referral code here. You can skip—this
          stays available on the Rewards page while you&apos;re eligible.
        </p>
      </div>

      {success ? (
        <div className="text-center py-4">
          <div className="w-12 h-12 bg-sage-50 rounded-full flex items-center justify-center mx-auto mb-3">
            <svg
              className="w-6 h-6 text-sage-500"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M5 13l4 4L19 7"
              />
            </svg>
          </div>
          <p className="text-sage-600 font-medium text-sm">Referral code applied.</p>
        </div>
      ) : (
        <>
          <div className="mb-4">
            <input
              type="text"
              value={code}
              onChange={e => setCode(e.target.value.toUpperCase())}
              onKeyDown={e => e.key === 'Enter' && void handleApply()}
              placeholder="Enter referral code"
              className="w-full px-4 py-3 bg-stone-50 border border-stone-200 rounded-xl text-center font-mono text-lg tracking-widest text-stone-900 placeholder:text-stone-400 placeholder:tracking-normal placeholder:font-sans placeholder:text-sm focus:outline-none focus:ring-2 focus:ring-primary-500/50 focus:border-primary-500/50 transition-all"
              disabled={isLoading}
            />
            {error ? <p className="text-coral-500 text-xs mt-2 text-center">{error}</p> : null}
          </div>

          <div className="space-y-2">
            <button
              type="button"
              onClick={() => void handleApply()}
              disabled={isLoading || !code.trim()}
              className="btn-primary w-full py-2.5 text-sm font-medium rounded-xl disabled:opacity-50 disabled:cursor-not-allowed">
              {isLoading ? 'Applying…' : 'Apply code'}
            </button>
            <button
              type="button"
              onClick={onNext}
              disabled={isLoading}
              className="w-full py-2.5 text-sm font-medium rounded-xl text-stone-400 hover:text-stone-700 transition-colors">
              Skip for now
            </button>
            <button
              type="button"
              onClick={onBack}
              disabled={isLoading}
              className="w-full py-2 text-sm text-stone-500 hover:text-stone-800 transition-colors">
              Back
            </button>
          </div>
        </>
      )}
    </div>
  );
};

export default ReferralApplyStep;
