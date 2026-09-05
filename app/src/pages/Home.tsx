import { useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import ConnectionIndicator from '../components/ConnectionIndicator';
import { DiscordBanner, PromotionalCreditsBanner } from '../components/home/HomeBanners';
import Button from '../components/ui/Button';
import { useUser } from '../hooks/useUser';
import { useT } from '../lib/i18n/I18nContext';
import { restartCoreProcess } from '../services/coreProcessControl';
import { selectBlockingState } from '../store/connectivitySelectors';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import { resolveTheme, setThemeMode, type ThemeMode } from '../store/themeSlice';
import { APP_VERSION } from '../utils/config';
import { resolveUserName } from '../utils/userName';

/** @deprecated Use `resolveUserName` from `utils/userName`. Kept for back-compat. */
export const resolveHomeUserName = resolveUserName;

type TranslateFn = (key: string, fallback?: string) => string;

function registryCardCopy(
  t: TranslateFn,
  blocking: 'internet-offline' | 'core-unreachable' | 'backend-only' | 'ok',
  coreError: string | undefined
) {
  if (blocking === 'core-unreachable') {
    const normalized = coreError?.toLowerCase() ?? '';
    if (normalized.includes('config missing')) {
      return {
        title: t('home.coreRegistriesBlockedTitle'),
        description: t('home.coreRegistriesBlockedMissingDescription'),
      };
    }
    if (normalized.includes('config invalid')) {
      return {
        title: t('home.coreRegistriesBlockedTitle'),
        description: t('home.coreRegistriesBlockedInvalidDescription'),
      };
    }
    return {
      title: t('home.coreRegistriesBlockedTitle'),
      description: t('home.coreRegistriesBlockedBridgeDescription'),
    };
  }

  if (blocking === 'internet-offline') {
    return {
      title: t('home.coreRegistriesBlockedTitle'),
      description: t('home.coreRegistriesBlockedOfflineDescription'),
    };
  }

  return { title: t('home.coreRegistries'), description: t('home.coreRegistriesDescription') };
}

const Home = () => {
  const { t } = useT();
  const { user } = useUser();
  const navigate = useNavigate();
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
  const connectivityErrors = useAppSelector(state => state.connectivity.lastError);
  const [isRestartingCore, setIsRestartingCore] = useState(false);
  const [restartError, setRestartError] = useState<string | null>(null);

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
  const registriesCard = registryCardCopy(t, blocking, connectivityErrors.core);

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
      <h1 className="min-h-14 text-32l font-bold text-content text-center">
        {typedWelcome}
        <span aria-hidden="true" className="ml-0.5 inline-block text-primary-500 animate-pulse">
          |
        </span>
      </h1>

      <div className="max-w-md w-full">
        {showPromoBanner && <PromotionalCreditsBanner promoCredits={promoCredits} />}

        {/* Main card — data-walkthrough target for step 1 */}
        <div
          data-walkthrough="home-card"
          className="bg-surface rounded-2xl shadow-soft border border-line p-6 animate-fade-up">
          {/* Header row: version centered, theme toggle right-aligned.
              The empty left spacer matches the toggle's width so the version
              stays visually centered. */}
          <div className="flex items-center justify-between mb-4">
            <div className="w-9" aria-hidden="true" />
            <span className="text-xs text-center text-content-faint">v{APP_VERSION}</span>
            <Button
              iconOnly
              variant="tertiary"
              onClick={toggleTheme}
              aria-label={isDark ? t('home.themeToggle.toLight') : t('home.themeToggle.toDark')}
              title={isDark ? t('home.themeToggle.toLight') : t('home.themeToggle.toDark')}
              className="rounded-full">
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
            </Button>
          </div>

          {/* Connection status */}
          <div className="flex justify-center mb-3">
            <ConnectionIndicator />
          </div>

          {/* Description — copy mirrors the active blocking state so the
              user never sees a "connected" message while the pill shows a
              failure. (#1527) */}
          <p className="text-sm text-content-muted text-center mb-6 leading-relaxed">
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
                className="w-full py-3 bg-amber-500 hover:bg-amber-600 disabled:opacity-50 text-content-inverted font-medium rounded-xl transition-colors duration-200">
                {isRestartingCore ? t('home.restartingCore') : t('home.restartCore')}
              </button>
              {restartError && (
                <p className="mt-2 text-xs text-coral-500 text-center">{restartError}</p>
              )}
            </div>
          )}

          {/* CTA button — data-walkthrough target for step 2 */}
          <Button
            data-walkthrough="home-cta"
            variant="primary"
            size="lg"
            onClick={handleStartCooking}
            disabled={blocking === 'core-unreachable' || blocking === 'internet-offline'}
            className="w-full">
            {t('home.askAssistant')}
          </Button>
        </div>

        <button
          type="button"
          onClick={() => navigate('/registries')}
          className="mt-3 w-full rounded-2xl border border-line bg-surface px-4 py-4 text-left shadow-soft transition-colors hover:border-primary-500/40 hover:bg-surface-muted">
          <div className="flex items-start gap-3">
            <div className="mt-0.5 flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-full bg-primary-500/10 text-primary-500">
              <svg
                className="h-5 w-5"
                fill="none"
                stroke="currentColor"
                strokeWidth={1.8}
                viewBox="0 0 24 24"
                aria-hidden="true">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  d="M4 7.5h16M4 12h16M4 16.5h16M6.75 5.25v13.5M12 5.25v13.5M17.25 5.25v13.5"
                />
              </svg>
            </div>
            <div className="min-w-0 flex-1">
              <div className="text-sm font-medium text-content">{registriesCard.title}</div>
              <div className="mt-1 text-xs leading-relaxed text-content-muted">
                {registriesCard.description}
              </div>
            </div>
          </div>
        </button>

        <button
          type="button"
          onClick={() => navigate('/workbench')}
          className="mt-3 w-full rounded-2xl border border-line bg-surface px-4 py-4 text-left shadow-soft transition-colors hover:border-primary-500/40 hover:bg-surface-muted">
          <div className="flex items-start gap-3">
            <div className="mt-0.5 flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-full bg-amber-500/10 text-amber-600">
              <svg
                className="h-5 w-5"
                fill="none"
                stroke="currentColor"
                strokeWidth={1.8}
                viewBox="0 0 24 24"
                aria-hidden="true">
                <path strokeLinecap="round" strokeLinejoin="round" d="M4 6h16M4 12h10M4 18h7" />
              </svg>
            </div>
            <div className="min-w-0 flex-1">
              <div className="text-sm font-medium text-content">{t('home.youpetWorkbench')}</div>
              <div className="mt-1 text-xs leading-relaxed text-content-muted">
                {t('home.youpetWorkbenchDescription')}
              </div>
            </div>
          </div>
        </button>

        <button
          type="button"
          onClick={() => navigate('/action-requests')}
          data-testid="home-action-request-inbox"
          className="mt-3 w-full rounded-2xl border border-line bg-surface px-4 py-4 text-left shadow-soft transition-colors hover:border-primary-500/40 hover:bg-surface-muted">
          <div className="flex items-start gap-3">
            <div className="mt-0.5 flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-full bg-sky-500/10 text-sky-600">
              <svg
                className="h-5 w-5"
                fill="none"
                stroke="currentColor"
                strokeWidth={1.8}
                viewBox="0 0 24 24"
                aria-hidden="true">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"
                />
              </svg>
            </div>
            <div className="min-w-0 flex-1">
              <div className="text-sm font-medium text-content">
                {t('home.youpetActionRequests')}
              </div>
              <div className="mt-1 text-xs leading-relaxed text-content-muted">
                {t('home.youpetActionRequestsDescription')}
              </div>
            </div>
          </div>
        </button>

        <DiscordBanner />

        {/* Next steps — compact directory of where to go next */}
        {/* <div className="mt-3 bg-surface rounded-2xl shadow-soft border border-line p-4">
          <div className="text-[11px] uppercase tracking-wide text-content-faint mb-2">Next steps</div>
          <div className="divide-y divide-line-subtle">
            <button
              onClick={() => navigate('/connections')}
              className="w-full flex items-center justify-between py-2.5 text-left hover:bg-surface-muted rounded-md px-2 -mx-2 transition-colors">
              <div>
                <div className="text-sm font-medium text-content">Connect your services</div>
                <div className="text-xs text-content-muted">
                  Give your assistant access to Gmail, Calendar, and more.
                </div>
              </div>
              <svg
                className="w-4 h-4 text-content-faint"
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
              className="w-full flex items-center justify-between py-2.5 text-left hover:bg-surface-muted rounded-md px-2 -mx-2 transition-colors">
              <div>
                <div className="text-sm font-medium text-content">Earn rewards</div>
                <div className="text-xs text-content-muted">
                  Unlock credits by using OpenHuman and completing milestones.
                </div>
              </div>
              <svg
                className="w-4 h-4 text-content-faint"
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
              className="w-full flex items-center justify-between py-2.5 text-left hover:bg-surface-muted rounded-md px-2 -mx-2 transition-colors">
              <div>
                <div className="text-sm font-medium text-content">Invite a friend</div>
                <div className="text-xs text-content-muted">
                  Share an invite — both of you get credits.
                </div>
              </div>
              <svg
                className="w-4 h-4 text-content-faint"
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
