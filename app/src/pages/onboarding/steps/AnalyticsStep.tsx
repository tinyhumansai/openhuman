import { useState } from 'react';

import { isTauri, openhumanUpdateAnalyticsSettings } from '../../../utils/tauriCommands';

interface AnalyticsStepProps {
  onNext: (analyticsEnabled: boolean) => void;
}

const AnalyticsStep = ({ onNext }: AnalyticsStepProps) => {
  const [selectedOption, setSelectedOption] = useState('shareAnalytics');

  const handleContinue = () => {
    const enabled = selectedOption === 'shareAnalytics';

    // Sync to core config so the Rust process also respects the setting
    if (isTauri()) {
      openhumanUpdateAnalyticsSettings({ enabled }).catch(() => {
        /* best-effort */
      });
    }

    onNext(enabled);
  };

  return (
    <div className="rounded-2xl border border-stone-200 bg-white p-8 shadow-soft animate-fade-up">
      <div className="text-center mb-4">
        <h1 className="text-xl font-bold mb-2 text-stone-900">Anonymized Analytics</h1>
        <p className="text-stone-600 text-sm">
          We collect fully anonymized usage data to help improve the app. No personal data,
          messages, or wallet keys are ever collected. You can change this anytime in Settings.
        </p>
      </div>

      <div className="space-y-4 mb-4">
        <div
          className={`p-4 rounded-xl border-2 cursor-pointer transition-all ${
            selectedOption === 'shareAnalytics'
              ? 'border-primary-500 bg-primary-50'
              : 'border-stone-200 bg-white hover:border-stone-300'
          }`}
          onClick={() => setSelectedOption('shareAnalytics')}>
          <div className="flex items-start space-x-4">
            <div className="flex items-center justify-center mt-0.5">
              <div
                className={`w-5 h-5 rounded-full border-2 flex items-center justify-center ${
                  selectedOption === 'shareAnalytics'
                    ? 'border-primary-500 bg-primary-500'
                    : 'border-stone-300 bg-white'
                }`}>
                {selectedOption === 'shareAnalytics' && (
                  <div className="w-2 h-2 bg-white rounded-full"></div>
                )}
              </div>
            </div>
            <div>
              <h3 className="font-semibold mb-1 text-sm text-stone-900">
                Share Anonymized Usage Data
              </h3>
              <p className="text-stone-600 text-xs leading-relaxed">
                Share anonymized crash reports and usage analytics to help us improve features and
                performance. All data is fully anonymized.
              </p>
            </div>
          </div>
        </div>

        <div
          className={`p-4 rounded-xl border-2 cursor-pointer transition-all ${
            selectedOption === 'maximumPrivacy'
              ? 'border-primary-500 bg-primary-50'
              : 'border-stone-200 bg-white hover:border-stone-300'
          }`}
          onClick={() => setSelectedOption('maximumPrivacy')}>
          <div className="flex items-start space-x-4">
            <div className="flex items-center justify-center mt-0.5">
              <div
                className={`w-5 h-5 rounded-full border-2 flex items-center justify-center ${
                  selectedOption === 'maximumPrivacy'
                    ? 'border-primary-500 bg-primary-500'
                    : 'border-stone-300 bg-white'
                }`}>
                {selectedOption === 'maximumPrivacy' && (
                  <div className="w-2 h-2 bg-white rounded-full"></div>
                )}
              </div>
            </div>
            <div>
              <h3 className="font-semibold mb-1 text-sm text-stone-900">Don't Collect Anything</h3>
              <p className="text-stone-600 text-xs leading-relaxed">
                We won't collect any usage analytics or crash reports. Keep all your data completely
                private.
              </p>
            </div>
          </div>
        </div>
      </div>

      <div className="p-4 bg-stone-50 rounded-xl border border-stone-200 mb-4">
        <div className="flex items-start space-x-2">
          <svg className="w-5 h-5 text-sage-500 mt-0.5" fill="currentColor" viewBox="0 0 20 20">
            <path
              fillRule="evenodd"
              d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-7-4a1 1 0 11-2 0 1 1 0 012 0zM9 9a1 1 0 000 2v3a1 1 0 001 1h1a1 1 0 100-2v-3a1 1 0 00-1-1H9z"
              clipRule="evenodd"
            />
          </svg>
          <div>
            <p className="font-medium text-sm text-stone-900">
              You can change this setting anytime
            </p>
            <p className="text-stone-600 text-xs mt-1">
              Your privacy preferences can be updated in Settings &gt; Privacy &amp; Security
            </p>
          </div>
        </div>
      </div>

      <button
        onClick={handleContinue}
        className="btn-primary w-full py-2.5 text-sm font-medium rounded-xl">
        Continue
      </button>
    </div>
  );
};

export default AnalyticsStep;
