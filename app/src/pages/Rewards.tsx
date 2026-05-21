import createDebug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import PillTabBar from '../components/PillTabBar';
import RewardsCommunityTab from '../components/rewards/RewardsCommunityTab';
import RewardsRedeemTab from '../components/rewards/RewardsRedeemTab';
import RewardsReferralsTab from '../components/rewards/RewardsReferralsTab';
import { useT } from '../lib/i18n/I18nContext';
import { rewardsApi } from '../services/api/rewardsApi';
import type { RewardsSnapshot } from '../types/rewards';

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
  const [selectedTab, setSelectedTab] = useState<RewardsTab>('rewards');
  const [snapshot, setSnapshot] = useState<RewardsSnapshot | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const loadRewards = useCallback(async (signal?: { cancelled: boolean }) => {
    log('fetching snapshot');
    setIsLoading(true);
    setError(null);
    try {
      const result = await rewardsApi.getMyRewards();
      if (signal?.cancelled) return;
      setSnapshot(result);
      log(
        'snapshot applied unlockedCount=%d totalCount=%d',
        result.summary.unlockedCount,
        result.summary.totalCount
      );
    } catch (err) {
      const message = errorMessage(err);
      log('snapshot load failed error=%s', message);
      if (signal?.cancelled) return;
      setSnapshot(null);
      setError(message);
    } finally {
      if (!signal?.cancelled) {
        setIsLoading(false);
      }
    }
  }, []);

  useEffect(() => {
    const signal = { cancelled: false };
    void loadRewards(signal);
    return () => {
      signal.cancelled = true;
    };
  }, [loadRewards]);

  const handleTabChange = useCallback((next: RewardsTab) => {
    log('tab changed next=%s', next);
    setSelectedTab(next);
  }, []);

  const handleRetry = useCallback(() => {
    log('retry requested');
    void loadRewards();
  }, [loadRewards]);

  return (
    <div className="min-h-full px-4 pt-6 pb-10">
      <div className="mx-auto max-w-2xl space-y-4">
        <PillTabBar
          items={[
            { label: t('rewards.referrals'), value: 'referrals' },
            { label: t('rewards.title'), value: 'rewards' },
            { label: t('rewards.coupons'), value: 'redeem' },
          ]}
          selected={selectedTab}
          onChange={handleTabChange}
          activeClassName="border-primary-600 bg-primary-600 text-white"
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
            snapshot={snapshot}
          />
        )}
      </div>
    </div>
  );
};

export default Rewards;
