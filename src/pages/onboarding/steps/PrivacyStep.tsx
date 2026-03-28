import { useState } from 'react';

interface PrivacyStepProps {
  onNext: (localModelConsentGiven: boolean) => void;
}

const PrivacyStep = ({ onNext }: PrivacyStepProps) => {
  const [decision, setDecision] = useState<boolean | null>(null);

  return (
    <div className="glass rounded-3xl p-8 shadow-large animate-fade-up">
      <div className="text-center mb-5">
        <h1 className="text-xl font-bold mb-2">Local Model Consent</h1>
        <p className="opacity-70 text-sm">
          Choose whether OpenHuman should use local model features on this device.
        </p>
      </div>

      <div className="space-y-3 mb-5">
        <div className="rounded-2xl border border-sage-500/30 bg-sage-500/10 p-3">
          <p className="text-sm font-medium mb-1">What you gain</p>
          <p className="text-xs opacity-80">
            Lower latency, higher privacy, and resilience when network connectivity is unstable.
          </p>
        </div>
        <div className="rounded-2xl border border-amber-500/30 bg-amber-500/10 p-3">
          <p className="text-sm font-medium mb-1">Resource impact</p>
          <p className="text-xs opacity-80">
            Typical setup needs 6-10 GB disk for model files and can use 4-8 GB RAM while running.
          </p>
        </div>
      </div>

      <div className="grid grid-cols-2 gap-2 mb-4">
        <button
          onClick={() => setDecision(true)}
          className={`py-2.5 text-sm font-medium rounded-xl border transition-colors ${
            decision === true
              ? 'border-sage-500 bg-sage-500/20 text-sage-200'
              : 'border-stone-600 hover:border-stone-500'
          }`}>
          I Consent
        </button>
        <button
          onClick={() => setDecision(false)}
          className={`py-2.5 text-sm font-medium rounded-xl border transition-colors ${
            decision === false
              ? 'border-amber-500 bg-amber-500/20 text-amber-200'
              : 'border-stone-600 hover:border-stone-500'
          }`}>
          Not Now
        </button>
      </div>

      <button
        onClick={() => decision !== null && onNext(decision)}
        disabled={decision === null}
        className="btn-primary w-full py-2.5 text-sm font-medium rounded-xl disabled:opacity-60 disabled:cursor-not-allowed">
        Continue
      </button>
    </div>
  );
};

export default PrivacyStep;
