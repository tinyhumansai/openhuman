import createDebug from 'debug';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { LuGift, LuUsers } from 'react-icons/lu';
import { useLocation, useNavigate } from 'react-router-dom';

import EmptyStateCard from '../components/EmptyStateCard';
import { SidebarContent } from '../components/layout/shell/SidebarSlot';
import TwoPaneNav from '../components/layout/TwoPaneNav';
import RewardsCommunityTab from '../components/rewards/RewardsCommunityTab';
import RewardsReferralsTab from '../components/rewards/RewardsReferralsTab';
import SettingsTabbedPage from '../components/settings/layout/SettingsTabbedPage';
import { useT } from '../lib/i18n/I18nContext';
import { useCoreState } from '../providers/CoreStateProvider';
import { rewardsApi } from '../services/api/rewardsApi';
import type { RewardsSnapshot } from '../types/rewards';
import { isLocalSessionToken } from '../utils/localSession';

/**
 * The two Rewards surfaces. Each is its own page with a sidebar entry rather
 * than a chip tab on one page: they share no state and none of them is a
 * refinement of another, so a tab row was hiding two destinations behind a
 * third. `?view=` is the address so a page survives a reload and can be linked.
 */
type RewardsView = 'rewards' | 'referrals';

const VIEWS: readonly RewardsView[] = ['rewards', 'referrals'] as const;

function isRewardsView(value: string): value is RewardsView {
  return (VIEWS as readonly string[]).includes(value);
}

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
  const [rewardsSnapshot, setRewardsSnapshot] = useState<RewardsSnapshot | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Unlike the pages that kept a Welcome landing, an unrecognised (or absent)
  // `?view=` lands on the main Rewards page rather than a fourth pseudo-view —
  // the sidebar always highlights exactly one of the three entries.
  const rawView = new URLSearchParams(location.search).get('view') ?? '';
  const view: RewardsView = isRewardsView(rawView) ? rawView : 'rewards';

  const setView = useCallback(
    (next: string) => {
      log('view changed next=%s', next);
      const params = new URLSearchParams(location.search);
      if (next === 'rewards') params.delete('view');
      else params.set('view', next);
      const search = params.toString();
      navigate({ pathname: location.pathname, search: search ? `?${search}` : '' });
    },
    [location.pathname, location.search, navigate]
  );

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

  const handleRetry = useCallback(() => {
    log('retry requested');
    void loadRewards();
  }, [loadRewards]);

  const nav = useMemo(
    () => (
      <SidebarContent>
        <div className="h-full overflow-hidden">
          <TwoPaneNav
            ariaLabel={t('rewards.title')}
            selected={view}
            onSelect={setView}
            groups={[
              {
                items: [
                  {
                    value: 'rewards',
                    label: t('rewards.title'),
                    icon: <LuGift className="h-4 w-4" />,
                  },
                  {
                    value: 'referrals',
                    label: t('rewards.referrals'),
                    icon: <LuUsers className="h-4 w-4" />,
                  },
                ],
              },
            ]}
          />
        </div>
      </SidebarContent>
    ),
    [t, view, setView]
  );

  if (isLocalSession) {
    return (
      <div className="h-full overflow-y-auto p-4">
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
          onAction={() => navigate('/settings/account')}
        />
      </div>
    );
  }

  const page =
    view === 'referrals'
      ? {
          title: t('rewards.referralSection.title'),
          description: t('rewards.referralSection.subtitle'),
          body: <RewardsReferralsTab />,
        }
      : {
          title: t('rewards.title'),
          description: t('rewards.header.desc'),
          body: (
            <RewardsCommunityTab
              error={error}
              isLoading={isLoading}
              onRetry={handleRetry}
              onSilentRefresh={handleSilentRefresh}
              snapshot={rewardsSnapshot}
            />
          ),
        };

  return (
    <>
      {nav}
      {/* `p-4` is the gutter SettingsTabbedPage's full-bleed divider bleeds
          through, and `h-full` is what makes the body actually scroll — the
          content surface is `overflow-hidden`, so a `min-h-full` page (what
          this was) grew past the card and clipped instead. */}
      <div className="h-full p-4" data-testid="rewards-page">
        <SettingsTabbedPage title={page.title} description={page.description}>
          <div className="space-y-4">{page.body}</div>
        </SettingsTabbedPage>
      </div>
    </>
  );
};

export default Rewards;
