import createDebug from 'debug';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import ConnectionIndicator from '../components/ConnectionIndicator';
import {
  DiscordBanner,
  EarlyBirdyBanner,
  PromotionalCreditsBanner,
  UsageLimitBanner,
} from '../components/home/HomeBanners';
import { dismissBanner, shouldShowBanner } from '../components/upsell/upsellDismissState';
import { useUsageState } from '../hooks/useUsageState';
import { useUser } from '../hooks/useUser';
import { useT } from '../lib/i18n/I18nContext';
import { restartCoreProcess } from '../services/coreProcessControl';
import { selectBlockingState } from '../store/connectivitySelectors';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import { resolveTheme, setThemeMode, type ThemeMode } from '../store/themeSlice';
import { APP_VERSION } from '../utils/config';
import { openhumanCronList } from '../utils/tauriCommands';

const homeLog = createDebug('app:home');

export function resolveHomeUserName(user: unknown): string {
  if (!user || typeof user !== 'object') return 'User';

  const record = user as Record<string, unknown>;
  const firstName =
    (typeof record.firstName === 'string' && record.firstName.trim()) ||
    (typeof record.first_name === 'string' && record.first_name.trim()) ||
    '';
  const lastName =
    (typeof record.lastName === 'string' && record.lastName.trim()) ||
    (typeof record.last_name === 'string' && record.last_name.trim()) ||
    '';
  const username = typeof record.username === 'string' ? record.username.trim() : '';
  const email = typeof record.email === 'string' ? record.email.trim() : '';

  const fullName = [firstName, lastName].filter(Boolean).join(' ').trim();
  if (fullName) return fullName;
  if (firstName) return firstName;
  if (username) return username.startsWith('@') ? username : `@${username}`;
  if (email) return email.split('@')[0] || 'User';
  return 'User';
}

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

  // Early birdy banner: once dismissed it stays gone (cooldown longer than any realistic session).
  const [showEarlyBirdy, setShowEarlyBirdy] = useState(() =>
    shouldShowBanner('home-earlybirdy', Number.MAX_SAFE_INTEGER)
  );

  const handleDismissEarlyBirdy = () => {
    dismissBanner('home-earlybirdy');
    setShowEarlyBirdy(false);
  };

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

  // Routine count for the card on home.
  const [activeRoutineCount, setActiveRoutineCount] = useState<number | null>(null);
  const loadRoutineCount = useCallback(async () => {
    try {
      const response = await openhumanCronList();
      const activeCount = response.result.filter(j => j.enabled).length;
      setActiveRoutineCount(activeCount);
    } catch (err) {
      homeLog('failed to load routine count', err);
    }
  }, []);
  useEffect(() => {
    void loadRoutineCount();
  }, [loadRoutineCount]);

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
      <div className="max-w-md w-full">
        {shouldShowBudgetCompletedMessage && (
          <UsageLimitBanner
            tone="danger"
            icon="⚠️"
            title={t('home.usageExhaustedTitle')}
            message={t('home.usageExhaustedBody')}
            ctaLabel={t('home.usageExhaustedCta')}
          />
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

          {/* Welcome title */}
          <h1 className="min-h-[3.5rem] text-32l font-bold text-stone-900 dark:text-neutral-100 text-center">
            {typedWelcome}
            <span aria-hidden="true" className="ml-0.5 inline-block text-primary-500 animate-pulse">
              |
            </span>
          </h1>

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

        {/* Routines card */}
        {activeRoutineCount !== null && (
          <button
            onClick={() => navigate('/routines')}
            className="mt-3 w-full bg-white dark:bg-neutral-900 rounded-2xl shadow-soft border border-stone-200 dark:border-neutral-800 p-4 flex items-center gap-3 text-left hover:border-primary-300 dark:hover:border-primary-500/40 transition-colors animate-fade-up">
            <div className="w-9 h-9 rounded-full bg-primary-50 dark:bg-primary-500/10 flex items-center justify-center flex-shrink-0">
              <svg
                className="w-4.5 h-4.5 text-primary-500"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={1.8}
                  d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"
                />
              </svg>
            </div>
            <div className="flex-1 min-w-0">
              <div className="text-sm font-medium text-stone-900 dark:text-neutral-100">
                {t('home.routinesCard')}
              </div>
              <div className="text-xs text-stone-500 dark:text-neutral-400">
                {activeRoutineCount > 0
                  ? t('home.routinesActive').replace('{count}', String(activeRoutineCount))
                  : t('routines.emptyHint')}
              </div>
            </div>
            <svg
              className="w-4 h-4 text-stone-400 dark:text-neutral-500 flex-shrink-0"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
            </svg>
          </button>
        )}

        {showEarlyBirdy && <EarlyBirdyBanner onDismiss={handleDismissEarlyBirdy} />}

        <DiscordBanner />

        {/* Next steps — compact directory of where to go next */}
        {/* <div className="mt-3 bg-white rounded-2xl shadow-soft border border-stone-200 p-4">
          <div className="text-[11px] uppercase tracking-wide text-stone-400 mb-2">Next steps</div>
          <div className="divide-y divide-stone-100">
            <button
              onClick={() => navigate('/skills')}
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
