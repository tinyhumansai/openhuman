import { useState } from 'react';

import { inviteApi } from '../../../services/api/inviteApi';

interface InviteCodeStepProps {
  onNext: () => void;
}

const InviteCodeStep = ({ onNext }: InviteCodeStepProps) => {
  const [code, setCode] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState(false);

  const handleRedeem = async () => {
    const trimmed = code.trim();
    if (!trimmed) return;

    setIsLoading(true);
    setError(null);

    try {
      await inviteApi.redeemInviteCode(trimmed);
      setSuccess(true);
      setTimeout(() => onNext(), 1500);
    } catch (err) {
      const msg =
        err && typeof err === 'object' && 'error' in err
          ? String((err as { error: string }).error)
          : 'Invalid or expired invite code';
      setError(msg);
    } finally {
      setIsLoading(false);
    }
  };

  return (
    <div className="rounded-2xl border border-stone-200 bg-white p-8 shadow-soft animate-fade-up">
      <div className="text-center mb-6">
        <h1 className="text-xl font-bold mb-2 text-stone-900">Have an Invite Code?</h1>
        <p className="text-stone-600 text-sm">
          Enter an invite code from a friend to unlock free credits. You can also skip this step.
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
          <p className="text-sage-500 font-medium text-sm">Invite code redeemed successfully!</p>
        </div>
      ) : (
        <>
          <div className="mb-4">
            <input
              type="text"
              value={code}
              onChange={e => setCode(e.target.value.toUpperCase())}
              onKeyDown={e => e.key === 'Enter' && handleRedeem()}
              placeholder="Enter invite code"
              className="w-full px-4 py-3 bg-stone-50 border border-stone-200 rounded-xl text-center font-mono text-lg tracking-widest text-stone-900 placeholder:text-stone-400 placeholder:tracking-normal placeholder:font-sans placeholder:text-sm focus:outline-none focus:ring-2 focus:ring-primary-500/50 focus:border-primary-500/50 transition-all"
              disabled={isLoading}
            />
            {error && <p className="text-coral-500 text-xs mt-2 text-center">{error}</p>}
          </div>

          <div className="space-y-2">
            <button
              onClick={handleRedeem}
              disabled={isLoading || !code.trim()}
              className="btn-primary w-full py-2.5 text-sm font-medium rounded-xl disabled:opacity-50 disabled:cursor-not-allowed">
              {isLoading ? 'Redeeming...' : 'Redeem Code'}
            </button>
            <button
              onClick={onNext}
              disabled={isLoading}
              className="w-full py-2.5 text-sm font-medium rounded-xl text-stone-400 hover:text-stone-700 transition-colors">
              Skip for now
            </button>
          </div>
        </>
      )}
    </div>
  );
};

export default InviteCodeStep;
