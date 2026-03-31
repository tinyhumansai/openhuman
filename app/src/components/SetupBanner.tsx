import { useCallback, useState } from 'react';

import { selectIsOnboarded, selectOnboardingDeferred } from '../store/authSelectors';
import { setOnboardingDeferredForUser } from '../store/authSlice';
import { useAppDispatch, useAppSelector } from '../store/hooks';

const SESSION_KEY = 'setupBannerDismissed';

/**
 * Non-intrusive banner shown when a user has deferred onboarding but hasn't completed it.
 * Provides a clear path to resume setup without blocking the app.
 */
const SetupBanner = () => {
  const dispatch = useAppDispatch();
  const isOnboarded = useAppSelector(selectIsOnboarded);
  const isDeferred = useAppSelector(selectOnboardingDeferred);
  const userId = useAppSelector(state => state.user.user?._id);

  const [sessionDismissed, setSessionDismissed] = useState(
    () => sessionStorage.getItem(SESSION_KEY) === 'true'
  );

  const handleResume = useCallback(() => {
    if (userId) {
      dispatch(setOnboardingDeferredForUser({ userId, deferred: false }));
    }
  }, [dispatch, userId]);

  const handleDismiss = useCallback(() => {
    sessionStorage.setItem(SESSION_KEY, 'true');
    setSessionDismissed(true);
  }, []);

  if (!isDeferred || isOnboarded || sessionDismissed || !userId) return null;

  return (
    <div className="mx-4 mt-3 mb-1 flex items-center justify-between gap-3 rounded-xl border border-primary-500/20 bg-primary-500/5 px-4 py-2.5">
      <div className="flex items-center gap-2.5 min-w-0">
        <svg
          className="w-4 h-4 text-primary-400 flex-shrink-0"
          viewBox="0 0 20 20"
          fill="currentColor">
          <path
            fillRule="evenodd"
            d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-7-4a1 1 0 11-2 0 1 1 0 012 0zM9 9a.75.75 0 000 1.5h.253a.25.25 0 01.244.304l-.459 2.066A1.75 1.75 0 0010.747 15H11a.75.75 0 000-1.5h-.253a.25.25 0 01-.244-.304l.459-2.066A1.75 1.75 0 009.253 9H9z"
            clipRule="evenodd"
          />
        </svg>
        <span className="text-sm text-stone-300 truncate">Finish setting up OpenHuman</span>
      </div>
      <div className="flex items-center gap-2 flex-shrink-0">
        <button
          onClick={handleResume}
          className="text-xs font-medium text-primary-400 hover:text-primary-300 transition-colors">
          Continue Setup
        </button>
        <button
          onClick={handleDismiss}
          className="p-0.5 text-stone-500 hover:text-stone-300 transition-colors"
          aria-label="Dismiss setup banner">
          <svg className="w-3.5 h-3.5" viewBox="0 0 16 16" fill="currentColor">
            <path d="M4.28 3.22a.75.75 0 00-1.06 1.06L6.94 8l-3.72 3.72a.75.75 0 101.06 1.06L8 9.06l3.72 3.72a.75.75 0 101.06-1.06L9.06 8l3.72-3.72a.75.75 0 00-1.06-1.06L8 6.94 4.28 3.22z" />
          </svg>
        </button>
      </div>
    </div>
  );
};

export default SetupBanner;
