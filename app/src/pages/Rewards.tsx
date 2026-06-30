import createDebug from 'debug';
import { useCallback, useEffect, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import EmptyStateCard from '../components/EmptyStateCard';
import ChipTabs from '../components/layout/ChipTabs';
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

  const loadRewards = useCallback(async (signal?: { cancelled: boolean }) => {
    log('fetching snapshot');
    setIsLoading(true);
    setError(null);
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
      log('snapshot load failed error=%s', message);
      if (signal?.cancelled) return;
      setRewardsSnapshot(null);
      setError(message);
    } finally {
      if (!signal?.cancelled) {
        setIsLoading(false);
      }
    }
  }, []);

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

  if (isLocalSession) {
    return (
      <div className="min-h-full px-4 pt-6 pb-10">
        <div className="mx-auto max-w-2xl space-y-4">
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
    <div className="min-h-full px-4 pt-6 pb-10">
      <div className="mx-auto max-w-2xl space-y-4">
        <ChipTabs<RewardsTab>
          items={[
            { id: 'referrals', label: t('rewards.referrals') },
            { id: 'rewards', label: t('rewards.title') },
            { id: 'redeem', label: t('rewards.coupons') },
          ]}
          value={selectedTab}
          onChange={handleTabChange}
          className="flex flex-wrap gap-2 pb-1"
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
            snapshot={rewardsSnapshot}
          />
        )}
      </div>
    </div>
  );
};

export default Rewards;
