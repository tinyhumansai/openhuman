import { useState } from 'react';

const DISMISSED_KEY = 'openhuman_beta_banner_dismissed';

const BetaBanner = () => {
  const [visible, setVisible] = useState(() => {
    try {
      return localStorage.getItem(DISMISSED_KEY) !== 'true';
    } catch {
      return true;
    }
  });

  if (!visible) return null;

  const handleDismiss = () => {
    try {
      localStorage.setItem(DISMISSED_KEY, 'true');
    } catch {
      // localStorage unavailable — dismiss for this session only
    }
    setVisible(false);
  };

  return (
    <div className="mb-4 flex items-start gap-3 rounded-xl border border-amber-200 bg-amber-50 px-4 py-3">
      {/* Beta pill */}
      <span className="mt-0.5 flex-shrink-0 rounded-md bg-amber-400 px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wide text-white">
        Beta
      </span>

      {/* Message */}
      <p className="flex-1 text-xs leading-relaxed text-stone-700">
        OpenHuman is in beta &mdash; you may hit rough edges. Your feedback helps us ship faster.
      </p>

      {/* Dismiss */}
      <button
        type="button"
        aria-label="Dismiss beta notice"
        onClick={handleDismiss}
        className="mt-0.5 flex-shrink-0 text-stone-400 hover:text-stone-600 transition-colors">
        <svg
          className="h-3.5 w-3.5"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
          aria-hidden="true">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M6 18L18 6M6 6l12 12"
          />
        </svg>
      </button>
    </div>
  );
};

export default BetaBanner;
