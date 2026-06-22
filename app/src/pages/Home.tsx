import { useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import ConnectionIndicator from '../components/ConnectionIndicator';
import {
  DiscordBanner,
  PromotionalCreditsBanner,
  UsageLimitBanner,
} from '../components/home/HomeBanners';
import { useUsageState } from '../hooks/useUsageState';
import { useUser } from '../hooks/useUser';
import { useT } from '../lib/i18n/I18nContext';
import { applyOpenRouterFreeModels } from '../services/api/openrouterFreeModels';
import { restartCoreProcess } from '../services/coreProcessControl';
import { selectBlockingState } from '../store/connectivitySelectors';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import { resolveTheme, setThemeMode, type ThemeMode } from '../store/themeSlice';
import { APP_VERSION } from '../utils/config';
import { resolveUserName } from '../utils/userName';

/** @deprecated Use `resolveUserName` from `utils/userName`. Kept for back-compat. */
export const resolveHomeUserName = resolveUserName;

const Home = () => {
  const { t } = useT();
  const { user } = useUser();
  const navigate = useNavigate();
  const { shouldShowBudgetCompletedMessage } = useUsageState();
  const _userName = resolveHomeUserName(user);
  const userName = _userName.split(' ')[0]; // Get first name only
  const promoCredits = user?.usage?.promotionBalanceUsd ?? 0;
  const isFreeTier =
    user?.subscription?.plan === 'FREE' || !user?.subscription?.hasActiveSubscription;
  const showPromoBanner = isFreeTier && promoCredits > 0.01;

  const welcomeVariants = useMemo(
    () => [`Welcome, ${userName} 👋`, `Let's cook, ${userName} 🧑‍🍳.`, `Time to Zone In 🧘🏻`],
    [userName]
  );
  const [welcomeVariantIndex, setWelcomeVariantIndex] = useState(0);
  const [typedWelcome, setTypedWelcome] = useState('');
  const [isDeletingWelcome, setIsDeletingWelcome] = useState(false);
  // 3-way blocking state (#1527) — internet > core > backend > ok. Each
  // failure mode now has its own copy so the user knows *which* link is
  // broken instead of seeing a single conflated "device offline" line.
  const blocking = useAppSelector(selectBlockingState);
  const [isRestartingCore, setIsRestartingCore] = useState(false);
  const [restartError, setRestartError] = useState<string | null>(null);
  const [openRouterStatus, setOpenRouterStatus] = useState<'idle' | 'saving' | 'error'>('idle');

  const dispatch = useAppDispatch();
  const themeMode = useAppSelector(state => state.theme.mode) as ThemeMode;
  const resolvedTheme = resolveTheme(themeMode);
  const isDark = resolvedTheme === 'dark';
  const toggleTheme = () => {
    dispatch(setThemeMode(isDark ? 'light' : 'dark'));
  };

  const handleRestartCore = async () => {
    setIsRestartingCore(true);
    setRestartError(null);
    try {
      await restartCoreProcess();
    } catch (err) {
      setRestartError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsRestartingCore(false);
    }
  };

  const handleUseOpenRouterFree = async () => {
    setOpenRouterStatus('saving');
    try {
      await applyOpenRouterFreeModels();
      setOpenRouterStatus('idle');
      navigate('/chat');
    } catch (err) {
      console.warn('[home] applyOpenRouterFreeModels failed', err);
      setOpenRouterStatus('error');
    }
  };

  const statusCopy = {
    ok: t('home.statusOk'),
    'backend-only': t('home.statusBackendOnly'),
    'core-unreachable': t('home.statusCoreUnreachable'),
    'internet-offline': t('home.statusInternetOffline'),
  }[blocking];

  // Open in-app chat.
  const handleStartCooking = async () => {
    navigate('/chat');
  };

  useEffect(() => {
    const activeVariant = welcomeVariants[welcomeVariantIndex] ?? '';
    const isFullyTyped = typedWelcome === activeVariant;
    const isFullyDeleted = typedWelcome.length === 0;

    const delay = isDeletingWelcome
      ? 36
      : isFullyTyped
        ? 1400
        : typedWelcome.length === 0
          ? 250
          : 55;

    const timeoutId = window.setTimeout(() => {
      if (!isDeletingWelcome) {
        if (isFullyTyped) {
          setIsDeletingWelcome(true);
          return;
        }

        setTypedWelcome(activeVariant.slice(0, typedWelcome.length + 1));
        return;
      }

      if (!isFullyDeleted) {
        setTypedWelcome(activeVariant.slice(0, typedWelcome.length - 1));
        return;
      }

      setIsDeletingWelcome(false);
      setWelcomeVariantIndex(current => (current + 1) % welcomeVariants.length);
    }, delay);

    return () => window.clearTimeout(timeoutId);
  }, [isDeletingWelcome, typedWelcome, welcomeVariantIndex, welcomeVariants]);

  return (
    <div className="min-h-full flex flex-col items-center justify-center p-4">
      {/* Welcome title */}
      <h1 className="min-h-[3.5rem] text-32l font-bold text-stone-900 dark:text-neutral-100 text-center">
        {typedWelcome}
        <span aria-hidden="true" className="ml-0.5 inline-block text-primary-500 animate-pulse">
          |
        </span>
      </h1>

      <div className="max-w-md w-full">
        {shouldShowBudgetCompletedMessage && (
          <UsageLimitBanner
            tone="danger"
            icon="⚠️"
            title={t('home.usageExhaustedTitle')}
            message={t('home.usageExhaustedBody')}
            ctaLabel={t('home.usageExhaustedCta')}
            secondaryCtaLabel={
              openRouterStatus === 'saving' ? t('openrouterFree.saving') : t('openrouterFree.cta')
            }
            onSecondaryCtaClick={() => {
              if (openRouterStatus !== 'saving') {
                void handleUseOpenRouterFree();
              }
            }}
          />
        )}
        {openRouterStatus === 'error' && (
          <div className="mb-3 rounded-lg border border-coral-200 bg-coral-50 px-3 py-2 text-xs text-coral-700 dark:border-coral-500/30 dark:bg-coral-900/20 dark:text-coral-200">
            {t('openrouterFree.error')}
          </div>
        )}

        {showPromoBanner && <PromotionalCreditsBanner promoCredits={promoCredits} />}

        {/* Main card — data-walkthrough target for step 1 */}
        <div
          data-walkthrough="home-card"
          className="bg-white dark:bg-neutral-900 rounded-2xl shadow-soft border border-stone-200 dark:border-neutral-800 p-6 animate-fade-up">
          {/* Header row: version centered, theme toggle right-aligned.
              The empty left spacer matches the toggle's width so the version
              stays visually centered. */}
          <div className="flex items-center justify-between mb-4">
            <div className="w-9" aria-hidden="true" />
            <span className="text-xs text-center text-stone-400 dark:text-neutral-500">
              v{APP_VERSION}
            </span>
            <button
              type="button"
              onClick={toggleTheme}
              aria-label={isDark ? t('home.themeToggle.toLight') : t('home.themeToggle.toDark')}
              title={isDark ? t('home.themeToggle.toLight') : t('home.themeToggle.toDark')}
              className="p-2 rounded-full text-stone-500 dark:text-neutral-400 hover:text-stone-700 dark:hover:text-neutral-200 hover:bg-stone-100 dark:hover:bg-neutral-800/60 transition-colors">
              {isDark ? (
                <svg
                  className="w-5 h-5"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth={2}
                  viewBox="0 0 24 24"
                  aria-hidden="true">
                  <circle cx="12" cy="12" r="4" />
                  <path
                    strokeLinecap="round"
                    d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M4.93 19.07l1.41-1.41M17.66 6.34l1.41-1.41"
                  />
                </svg>
              ) : (
                <svg
                  className="w-5 h-5"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth={2}
                  viewBox="0 0 24 24"
                  aria-hidden="true">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79Z"
                  />
                </svg>
              )}
            </button>
          </div>

          {/* Connection status */}
          <div className="flex justify-center mb-3">
            <ConnectionIndicator />
          </div>

          {/* Description — copy mirrors the active blocking state so the
              user never sees a "connected" message while the pill shows a
              failure. (#1527) */}
          <p className="text-sm text-stone-500 dark:text-neutral-400 text-center mb-6 leading-relaxed">
            {statusCopy}
          </p>

          {/* Recovery action: only shown when the local core sidecar is
              the broken link — internet/backend outages are not actionable
              from here. */}
          {blocking === 'core-unreachable' && (
            <div className="mb-4">
              <button
                onClick={handleRestartCore}
                disabled={isRestartingCore}
                className="w-full py-3 bg-amber-500 hover:bg-amber-600 disabled:opacity-50 text-white font-medium rounded-xl transition-colors duration-200">
                {isRestartingCore ? t('home.restartingCore') : t('home.restartCore')}
              </button>
              {restartError && (
                <p className="mt-2 text-xs text-coral-500 text-center">{restartError}</p>
              )}
            </div>
          )}

          {/* CTA button — data-walkthrough target for step 2 */}
          <button
            data-walkthrough="home-cta"
            onClick={handleStartCooking}
            disabled={blocking === 'core-unreachable' || blocking === 'internet-offline'}
            className="w-full py-3 bg-primary-500 hover:bg-primary-600 disabled:opacity-50 disabled:cursor-not-allowed text-white font-medium rounded-xl transition-colors duration-200">
            {t('home.askAssistant')}
          </button>
        </div>

        <DiscordBanner />

        {/* Next steps — compact directory of where to go next */}
        {/* <div className="mt-3 bg-white rounded-2xl shadow-soft border border-stone-200 p-4">
          <div className="text-[11px] uppercase tracking-wide text-stone-400 mb-2">Next steps</div>
          <div className="divide-y divide-stone-100">
            <button
              onClick={() => navigate('/connections')}
              className="w-full flex items-center justify-between py-2.5 text-left hover:bg-stone-50 rounded-md px-2 -mx-2 transition-colors">
              <div>
                <div className="text-sm font-medium text-stone-900">Connect your services</div>
                <div className="text-xs text-stone-500">
                  Give your assistant access to Gmail, Calendar, and more.
                </div>
              </div>
              <svg
                className="w-4 h-4 text-stone-400"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M9 5l7 7-7 7"
                />
              </svg>
            </button>
            <button
              onClick={() => navigate('/rewards')}
              className="w-full flex items-center justify-between py-2.5 text-left hover:bg-stone-50 rounded-md px-2 -mx-2 transition-colors">
              <div>
                <div className="text-sm font-medium text-stone-900">Earn rewards</div>
                <div className="text-xs text-stone-500">
                  Unlock credits by using OpenHuman and completing milestones.
                </div>
              </div>
              <svg
                className="w-4 h-4 text-stone-400"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M9 5l7 7-7 7"
                />
              </svg>
            </button>
            <button
              onClick={() => navigate('/invites')}
              className="w-full flex items-center justify-between py-2.5 text-left hover:bg-stone-50 rounded-md px-2 -mx-2 transition-colors">
              <div>
                <div className="text-sm font-medium text-stone-900">Invite a friend</div>
                <div className="text-xs text-stone-500">
                  Share an invite — both of you get credits.
                </div>
              </div>
              <svg
                className="w-4 h-4 text-stone-400"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M9 5l7 7-7 7"
                />
              </svg>
            </button>
          </div>
        </div> */}
      </div>
    </div>
  );
};

export default Home;
