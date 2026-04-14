import { useEffect, useState } from 'react';

import PillTabBar from '../components/PillTabBar';
import RewardsCommunityTab from '../components/rewards/RewardsCommunityTab';
import RewardsRedeemTab from '../components/rewards/RewardsRedeemTab';
import RewardsReferralsTab from '../components/rewards/RewardsReferralsTab';
import { rewardsApi } from '../services/api/rewardsApi';
import type { RewardsSnapshot } from '../types/rewards';

type RewardsTab = 'referrals' | 'redeem' | 'rewards';

const Rewards = () => {
  const [selectedTab, setSelectedTab] = useState<RewardsTab>('rewards');
  const [snapshot, setSnapshot] = useState<RewardsSnapshot | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    const loadRewards = async () => {
      try {
        const result = await rewardsApi.getMyRewards();
        if (!cancelled) {
          setSnapshot(result);
          console.debug('[rewards] snapshot applied', {
            unlockedCount: result.summary.unlockedCount,
            totalCount: result.summary.totalCount,
          });
        }
      } catch (err) {
        const message =
          err && typeof err === 'object' && 'error' in err && typeof err.error === 'string'
            ? err.error
            : err instanceof Error
              ? err.message
              : 'Unable to load rewards';
        console.debug('[rewards] snapshot load failed', message);
        if (!cancelled) {
          setSnapshot(null);
          setError(message);
        }
      } finally {
        if (!cancelled) {
          setIsLoading(false);
        }
      }
    };

    void loadRewards();

    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <div className="min-h-full px-4 pt-6 pb-10">
      <div className="mx-auto max-w-2xl space-y-4">
        <PillTabBar
          items={[
            { label: 'Referrals', value: 'referrals' },
            { label: 'Rewards', value: 'rewards' },
            { label: 'Redeem', value: 'redeem' },
          ]}
          selected={selectedTab}
          onChange={setSelectedTab}
          activeClassName="border-primary-600 bg-primary-600 text-white"
        />

        {selectedTab === 'referrals' ? (
          <RewardsReferralsTab />
        ) : selectedTab === 'redeem' ? (
          <RewardsRedeemTab />
        ) : (
          <RewardsCommunityTab error={error} isLoading={isLoading} snapshot={snapshot} />
        )}
      </div>
    </div>
  );
};

export default Rewards;
