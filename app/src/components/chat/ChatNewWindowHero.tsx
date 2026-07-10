import debugFactory from 'debug';
import { useEffect, useMemo, useState } from 'react';

import { useUsageState } from '../../hooks/useUsageState';
import { useUser } from '../../hooks/useUser';
import { useT } from '../../lib/i18n/I18nContext';
import { applyOpenRouterFreeModels } from '../../services/api/openrouterFreeModels';
import { restartCoreProcess } from '../../services/coreProcessControl';
import { selectBlockingState } from '../../store/connectivitySelectors';
import { useAppSelector } from '../../store/hooks';
import { resolveUserName } from '../../utils/userName';
import { DiscordBanner, PromotionalCreditsBanner, UsageLimitBanner } from '../home/HomeBanners';

const debug = debugFactory('chat:new-window-hero');

/**
 * Hero shown above the composer in the chat "new window" (empty thread) state —
 * the merged Home surface. Mirrors the former Home card (greeting, connection
 * status, version + light/dark toggle, banners), but drops the "Ask Assistant"
 * CTA: the composer directly below is the call to action now. The
 * core-unreachable recovery button is preserved since the composer is disabled
 * while the core is down.
 */
export default function ChatNewWindowHero() {
  const { t } = useT();
  const { user } = useUser();
  const { shouldShowBudgetCompletedMessage } = useUsageState();

  const userName = resolveUserName(user).split(' ')[0];
  const promoCredits = user?.usage?.promotionBalanceUsd ?? 0;
  const isFreeTier =
    user?.subscription?.plan === 'FREE' || !user?.subscription?.hasActiveSubscription;
  const showPromoBanner = isFreeTier && promoCredits > 0.01;

  const blocking = useAppSelector(selectBlockingState);

  const [isRestartingCore, setIsRestartingCore] = useState(false);
  const [restartError, setRestartError] = useState<string | null>(null);
  const [openRouterStatus, setOpenRouterStatus] = useState<'idle' | 'saving' | 'error'>('idle');

  const welcomeVariants = useMemo(
    () => [
      t('chat.newWindowWelcome1').replace('{name}', userName),
      t('chat.newWindowWelcome2').replace('{name}', userName),
      t('chat.newWindowWelcome3').replace('{name}', userName),
    ],
    [t, userName]
  );
  const [welcomeVariantIndex, setWelcomeVariantIndex] = useState(0);
  const [typedWelcome, setTypedWelcome] = useState('');
  const [isDeletingWelcome, setIsDeletingWelcome] = useState(false);

  const statusCopy = {
    ok: t('home.statusOk'),
    'backend-only': t('home.statusBackendOnly'),
    'core-unreachable': t('home.statusCoreUnreachable'),
    'internet-offline': t('home.statusInternetOffline'),
  }[blocking];

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
    } catch (err) {
      debug('applyOpenRouterFreeModels failed: %o', err);
      setOpenRouterStatus('error');
    }
  };

  // Typewriter cycle — identical cadence to the former Home greeting.
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
    <div className="mx-auto flex h-full w-full max-w-md flex-col justify-center py-4">
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

      {/* Main card — sizes to its content. The full height lives on the
          container (this column is h-full and centers the card), so the
          composer stays pinned at the bottom of the surface. ~80% tint over
          the app background. */}
      <div
        data-walkthrough="home-card"
        className="animate-fade-up rounded-2xl border border-line/80 bg-surface/80 p-6 shadow-soft backdrop-blur-sm dark:border-line/80">
        {/* Animated greeting */}
        <h1 className="min-h-[3.5rem] text-2xl text-center font-bold text-content">
          {typedWelcome}
          <span aria-hidden="true" className="ml-0.5 inline-block animate-pulse text-primary-500">
            |
          </span>
        </h1>

        {/* Description — copy mirrors the active blocking state (incl. the
            "device connected" get-started line in the normal case). */}
        <p className="text-center text-sm leading-relaxed text-content-muted">{statusCopy}</p>

        {/* Recovery: only when the local core is the broken link. */}
        {blocking === 'core-unreachable' && (
          <div className="mt-4">
            <button
              type="button"
              onClick={handleRestartCore}
              disabled={isRestartingCore}
              className="w-full rounded-xl bg-amber-500 py-3 font-medium text-content-inverted transition-colors duration-200 hover:bg-amber-600 disabled:opacity-50">
              {isRestartingCore ? t('home.restartingCore') : t('home.restartCore')}
            </button>
            {restartError && (
              <p className="mt-2 text-center text-xs text-coral-500">{restartError}</p>
            )}
          </div>
        )}
      </div>

      <DiscordBanner />
    </div>
  );
}
