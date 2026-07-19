import createDebug from 'debug';
import { useCallback, useEffect, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import EmptyStateCard from '../components/EmptyStateCard';
import ChipTabs from '../components/layout/ChipTabs';
import PageSectionHeader from '../components/layout/PageSectionHeader';
import PageWelcome from '../components/layout/PageWelcome';
import { usePageWelcomeView } from '../components/layout/usePageWelcomeView';
import RewardsCommunityTab from '../components/rewards/RewardsCommunityTab';
import RewardsRedeemTab from '../components/rewards/RewardsRedeemTab';
import RewardsReferralsTab from '../components/rewards/RewardsReferralsTab';
import { settingsNavState } from '../components/settings/modal/settingsOverlay';
import { useT } from '../lib/i18n/I18nContext';
import { useCoreState } from '../providers/CoreStateProvider';
import { rewardsApi } from '../services/api/rewardsApi';
import type { RewardsSnapshot } from '../types/rewards';
import { isLocalSessionToken } from '../utils/localSession';

type RewardsTab = 'referrals' | 'redeem' | 'rewards';

const log = createDebug('rewards');

function errorMessage(err: unknown): string {
  if (err && typeof err === 'object' && 'error' in err && typeof err.error === 'string') {
    return err.error;
  }
  if (err instanceof Error) {
    return err.message;
  }
  return 'Unable to load rewards'; // fallback — translated at call site
}

const Rewards = () => {
  const { t } = useT();
  const navigate = useNavigate();
  const location = useLocation();
  const { snapshot: coreSnapshot } = useCoreState();
  const isLocalSession = isLocalSessionToken(coreSnapshot.sessionToken);
  const [selectedTab, setSelectedTab] = useState<RewardsTab>('rewards');
  const [rewardsSnapshot, setRewardsSnapshot] = useState<RewardsSnapshot | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const loadRewards = useCallback(
    async (signal?: { cancelled: boolean }, opts?: { silent?: boolean }) => {
      const silent = opts?.silent === true;
      log('fetching snapshot silent=%s', silent);
      // A silent refresh (e.g. reconciling after a claim) keeps the current view
      // and never flips into the loading/error state — a failed background refetch
      // must not blank a page whose data is still valid.
      if (!silent) {
        setIsLoading(true);
        setError(null);
      }
      try {
        const result = await rewardsApi.getMyRewards();
        if (signal?.cancelled) return;
        setRewardsSnapshot(result);
        log(
          'snapshot applied unlockedCount=%d totalCount=%d',
          result.summary.unlockedCount,
          result.summary.totalCount
        );
      } catch (err) {
        const message = errorMessage(err);
        log('snapshot load failed silent=%s error=%s', silent, message);
        if (signal?.cancelled || silent) return;
        setRewardsSnapshot(null);
        setError(message);
      } finally {
        if (!signal?.cancelled && !silent) {
          setIsLoading(false);
        }
      }
    },
    []
  );

  const handleSilentRefresh = useCallback(
    () => loadRewards(undefined, { silent: true }),
    [loadRewards]
  );

  useEffect(() => {
    if (isLocalSession) {
      return;
    }
    const signal = { cancelled: false };
    void loadRewards(signal);
    return () => {
      signal.cancelled = true;
    };
  }, [isLocalSession, loadRewards]);

  // After a Discord (or any) OAuth connect completes, the deep-link listener dispatches
  // `oauth:success` — refresh the snapshot so the Discord connection / username updates live.
  useEffect(() => {
    if (isLocalSession) {
      return;
    }
    const handleOAuthSuccess = () => {
      log('oauth success event received; refreshing rewards snapshot');
      void loadRewards();
    };
    window.addEventListener('oauth:success', handleOAuthSuccess);
    return () => {
      window.removeEventListener('oauth:success', handleOAuthSuccess);
    };
  }, [isLocalSession, loadRewards]);

  const handleTabChange = useCallback((next: RewardsTab) => {
    log('tab changed next=%s', next);
    setSelectedTab(next);
  }, []);

  const handleRetry = useCallback(() => {
    log('retry requested');
    void loadRewards();
  }, [loadRewards]);

  const { view, setView, nav } = usePageWelcomeView({
    ariaLabel: t('rewards.title'),
    welcomeLabel: t('rewards.welcome.nav'),
    mainLabel: t('rewards.welcome.main'),
    mainIconPath:
      'M12 8v8m0-8l-3-3m3 3l3-3M8 14H6a2 2 0 01-2-2V7a2 2 0 012-2h2m8 9h2a2 2 0 002-2V7a2 2 0 00-2-2h-2M7 19h10',
  });

  if (view === 'welcome' && !isLocalSession) {
    return (
      <>
        {nav}
        <PageWelcome
          testId="rewards-welcome"
          accent="coral"
          icon="🎁"
          eyebrow={t('rewards.welcome.eyebrow')}
          title={t('rewards.welcome.title')}
          description={t('rewards.welcome.body')}
          ctas={[
            {
              label: t('rewards.welcome.ctaView'),
              icon: '🎁',
              onClick: () => setView('main'),
              testId: 'rewards-welcome-cta-view',
            },
          ]}
          featuresHeading={t('rewards.welcome.featsLabel')}
          features={[
            {
              icon: '⭐',
              title: t('rewards.welcome.feat1Title'),
              description: t('rewards.welcome.feat1Body'),
            },
            {
              icon: '🔥',
              title: t('rewards.welcome.feat2Title'),
              description: t('rewards.welcome.feat2Body'),
            },
            {
              icon: '🎟️',
              title: t('rewards.welcome.feat3Title'),
              description: t('rewards.welcome.feat3Body'),
            },
          ]}
        />
      </>
    );
  }

  if (isLocalSession) {
    return (
      <div className="min-h-full px-4 pt-6 pb-10">
        <div className="mx-auto max-w-3xl space-y-4">
          <EmptyStateCard
            className="shadow-soft"
            icon={
              <svg
                className="h-7 w-7 text-primary-500"
                fill="none"
                viewBox="0 0 24 24"
                stroke="currentColor"
                strokeWidth={1.5}
                aria-hidden="true">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  d="M12 8v8m0-8l-3-3m3 3l3-3M8 14H6a2 2 0 01-2-2V7a2 2 0 012-2h2m8 9h2a2 2 0 002-2V7a2 2 0 00-2-2h-2M7 19h10"
                />
              </svg>
            }
            title={t('rewards.title')}
            description={t('rewards.localUnavailable')}
            actionLabel={t('rewards.localUnavailableCta')}
            onAction={() => navigate('/settings/account', settingsNavState(location))}
          />
        </div>
      </div>
    );
  }

  return (
    <>
      {nav}
      <div className="min-h-full px-4 pt-6 pb-10">
        <div className="mx-auto max-w-3xl space-y-4">
          <PageSectionHeader
            title={t('rewards.title')}
            description={t('rewards.header.desc')}
            tabs={
              <ChipTabs<RewardsTab>
                items={[
                  { id: 'referrals', label: t('rewards.referrals') },
                  { id: 'rewards', label: t('rewards.title') },
                  { id: 'redeem', label: t('rewards.coupons') },
                ]}
                value={selectedTab}
                onChange={handleTabChange}
                className="flex flex-wrap gap-1.5"
              />
            }
          />

          {selectedTab === 'referrals' ? (
            <RewardsReferralsTab />
          ) : selectedTab === 'redeem' ? (
            <RewardsRedeemTab />
          ) : (
            <RewardsCommunityTab
              error={error}
              isLoading={isLoading}
              onRetry={handleRetry}
              onSilentRefresh={handleSilentRefresh}
              snapshot={rewardsSnapshot}
            />
          )}
        </div>
      </div>
    </>
  );
};

export default Rewards;
