import { useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import ConnectionIndicator from '../components/ConnectionIndicator';
import {
  DiscordBanner,
  EarlyBirdyBanner,
  PromotionalCreditsBanner,
  UsageLimitBanner,
} from '../components/home/HomeBanners';
import { useUsageState } from '../hooks/useUsageState';
import { useUser } from '../hooks/useUser';
import { useAppSelector } from '../store/hooks';
import { selectSocketStatus } from '../store/socketSelectors';
import { APP_VERSION } from '../utils/config';

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
  const { user } = useUser();
  const navigate = useNavigate();
  const { isRateLimited, shouldShowBudgetCompletedMessage } = useUsageState();
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
  // Mirror the same socket status the `ConnectionIndicator` pill consumes
  // so the description copy below the pill never contradicts it (the old
  // hard-coded "connected" message lied while the pill said "Connecting"
  // / "Disconnected").
  const socketStatus = useAppSelector(selectSocketStatus);
  const statusCopy = {
    connected:
      'Your device is connected. Keep the app running to keep the connection alive. Message your assistant with the button below.',
    connecting: 'Connecting. Hang tight, this usually takes a second.',
    disconnected:
      'Your device is offline right now. Check your network or restart the app to reconnect.',
  }[socketStatus];

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
        {isRateLimited && (
          <UsageLimitBanner
            tone="warning"
            icon="⏳"
            title="You’ve Hit Your Limits"
            message="You’ve reached your short-term usage cap. Buy top-up credits to keep going right away."
            ctaLabel="Buy top-up credits"
          />
        )}

        {!isRateLimited && shouldShowBudgetCompletedMessage && (
          <UsageLimitBanner
            tone="danger"
            icon="⚠️"
            title="You’ve Exhausted Your Usage"
            message="You’re out of included usage for now. Start a subscription to unlock more ongoing capacity."
            ctaLabel="Get a subscription"
          />
        )}

        {showPromoBanner && <PromotionalCreditsBanner promoCredits={promoCredits} />}

        {/* Main card */}
        <div className="bg-white rounded-2xl shadow-soft border border-stone-200 p-6 animate-fade-up">
          {/* Header row: logo + version + settings */}
          <div className="flex items-center justify-center mb-4">
            <span className="text-xs text-center text-stone-400">v{APP_VERSION}</span>
          </div>

          {/* Welcome title */}
          <h1 className="min-h-[3.5rem] text-32l font-bold text-stone-900 text-center">
            {typedWelcome}
            <span aria-hidden="true" className="ml-0.5 inline-block text-primary-500 animate-pulse">
              |
            </span>
          </h1>

          {/* Connection status */}
          <div className="flex justify-center mb-3">
            <ConnectionIndicator />
          </div>

          {/* Description — mirrors the pill's socket status to avoid
              telling the user they're connected while the pill shows
              "Connecting" / "Disconnected". */}
          <p className="text-sm text-stone-500 text-center mb-6 leading-relaxed">{statusCopy}</p>

          {/* CTA button */}
          <button
            onClick={handleStartCooking}
            className="w-full py-3 bg-primary-500 hover:bg-primary-600 text-white font-medium rounded-xl transition-colors duration-200">
            Message OpenHuman
          </button>
        </div>

        <EarlyBirdyBanner />

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
