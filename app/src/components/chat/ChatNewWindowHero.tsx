import { useEffect, useMemo, useState } from 'react';

import { useUser } from '../../hooks/useUser';
import { useT } from '../../lib/i18n/I18nContext';
import { restartCoreProcess } from '../../services/coreProcessControl';
import { selectBlockingState } from '../../store/connectivitySelectors';
import { useAppSelector } from '../../store/hooks';
import { resolveUserName } from '../../utils/userName';
import { DiscordBanner, PromotionalCreditsBanner } from '../home/HomeBanners';
import { Button } from '../ui';

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

  const userName = resolveUserName(user).split(' ')[0];
  const promoCredits = user?.usage?.promotionBalanceUsd ?? 0;
  const isFreeTier =
    user?.subscription?.plan === 'FREE' || !user?.subscription?.hasActiveSubscription;
  const showPromoBanner = isFreeTier && promoCredits > 0.01;

  const blocking = useAppSelector(selectBlockingState);

  const [isRestartingCore, setIsRestartingCore] = useState(false);
  const [restartError, setRestartError] = useState<string | null>(null);

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
      {showPromoBanner && <PromotionalCreditsBanner promoCredits={promoCredits} />}

      {/* Main card — sizes to its content. The full height lives on the
          container (this column is h-full and centers the card), so the
          composer stays pinned at the bottom of the surface. ~80% tint over
          the app background. */}
      <div
        data-walkthrough="home-card"
        // `surface-muted`, not `surface/80`: the translucent fill only read as a
        // card while the chat page painted a darker tint beneath it. The page is
        // the card surface now, so surface/80 over surface would flatten to the
        // same colour and leave only the border. This is the same lift token the
        // message bubbles use.
        className="animate-fade-up rounded-2xl border border-line/80 bg-surface-muted p-6 shadow-soft dark:border-line/80">
        {/* Animated greeting */}
        <h1 className="min-h-14 text-2xl text-center font-bold text-content">
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
            <Button
              size="lg"
              onClick={handleRestartCore}
              disabled={isRestartingCore}
              className="w-full rounded-xl bg-amber-500 text-content-inverted hover:bg-amber-600">
              {isRestartingCore ? t('home.restartingCore') : t('home.restartCore')}
            </Button>
            {restartError && (
              <p className="mt-2 text-center text-xs text-coral-500">{restartError}</p>
            )}
          </div>
        )}
      </div>

      {/* Prompt heading — sits directly above the composer, which is the call
          to action. This is the string the composer placeholder used to carry
          (`chat.typeMessage`); the placeholder is now the plain
          "Send a message" affordance, so the question moved here where it can
          be a real heading rather than hint text inside an input. */}
      <h2 className="mt-6 text-center text-lg font-semibold text-content">
        {t('chat.newWindowPrompt')}
      </h2>

      <DiscordBanner />
    </div>
  );
}
