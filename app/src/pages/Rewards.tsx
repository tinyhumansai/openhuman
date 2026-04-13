import { useEffect, useState } from 'react';

import RewardsCommunityTab from '../components/rewards/RewardsCommunityTab';
import RewardsReferralsTab from '../components/rewards/RewardsReferralsTab';
import { useUser } from '../hooks/useUser';
import { rewardsApi } from '../services/api/rewardsApi';
import type { RewardsSnapshot } from '../types/rewards';

type RewardsTab = 'referrals' | 'rewards';

const Rewards = () => {
  const { user } = useUser();
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
      <div className="mx-auto max-w-lg space-y-4">
        <div className="flex gap-2 overflow-x-auto pb-1 scrollbar-hide">
          {([
            ['referrals', 'Referrals'],
            ['rewards', 'Rewards'],
          ] as const).map(([tab, label]) => (
            <button
              key={tab}
              type="button"
              aria-pressed={selectedTab === tab}
              onClick={() => setSelectedTab(tab)}
              className={`flex-shrink-0 rounded-full border px-3 py-1 text-xs font-medium transition-colors ${
                selectedTab === tab
                  ? 'border-primary-600 bg-primary-600 text-white'
                  : 'border-stone-200 bg-white text-stone-600 hover:bg-stone-50'
              }`}>
              {label}
            </button>
          ))}
        </div>

        {selectedTab === 'referrals' ? (
          <RewardsReferralsTab />
        ) : (
          <RewardsCommunityTab
            error={error}
            isLoading={isLoading}
            onSelectReferrals={() => setSelectedTab('referrals')}
            plan={snapshot?.summary.plan ?? user?.subscription?.plan ?? 'FREE'}
            snapshot={snapshot}
          />
        )}
      </div>
    </div>
  );
};

export default Rewards;
