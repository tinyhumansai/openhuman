import { useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { DISCORD_INVITE_URL } from '../../../utils/links';

const DISMISSED_KEY = 'openhuman_beta_banner_dismissed';

const BetaBanner = () => {
  const { t } = useT();
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
      {/* Message */}
      <p className="flex-1 text-xs leading-relaxed text-stone-700">
        {t('misc.beta')}{' '}
        <a
          href={DISCORD_INVITE_URL}
          target="_blank"
          rel="noopener noreferrer"
          aria-label={t('common.learnMore')}
          className="font-medium text-amber-800 underline underline-offset-2 hover:text-amber-900">
          {t('common.learnMore')}
        </a>{' '}
        {t('onboarding.welcomeDesc')}
      </p>

      {/* Dismiss */}
      <button
        type="button"
        aria-label={t('common.dismiss')}
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
