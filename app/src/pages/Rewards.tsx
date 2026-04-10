import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import ReferralRewardsSection from '../components/referral/ReferralRewardsSection';
import RewardsCouponSection from '../components/rewards/RewardsCouponSection';
import { useUser } from '../hooks/useUser';
import { rewardsApi } from '../services/api/rewardsApi';
import type {
  RewardsAchievement,
  RewardsDiscordRoleStatus,
  RewardsSnapshot,
} from '../types/rewards';
import { DISCORD_INVITE_URL } from '../utils/links';

function discordMembershipLabel(snapshot: RewardsSnapshot | null): string {
  if (!snapshot) return 'Waiting for backend sync';
  switch (snapshot.discord.membershipStatus) {
    case 'member':
      return 'Joined the server';
    case 'not_in_guild':
      return 'Linked, but not in server';
    case 'not_linked':
      return 'Not linked';
    default:
      return 'Membership status unavailable';
  }
}

function roleStatusLabel(status: RewardsDiscordRoleStatus): string {
  switch (status) {
    case 'assigned':
      return 'Assigned in Discord';
    case 'not_assigned':
      return 'Earned, pending Discord assignment';
    case 'not_linked':
      return 'Link Discord to receive this role';
    case 'not_in_guild':
      return 'Join the server to receive this role';
    case 'not_configured':
      return 'Discord role not configured';
    default:
      return 'Role sync status unavailable';
  }
}

function formatNumber(value: number): string {
  return new Intl.NumberFormat('en-US').format(Math.max(0, Math.trunc(value)));
}

const Rewards = () => {
  const navigate = useNavigate();
  const { user } = useUser();
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

  const rewardRoles: RewardsAchievement[] = snapshot?.achievements ?? [];
  const unlocked = snapshot?.summary.unlockedCount ?? rewardRoles.filter(role => role.unlocked).length;
  const total = snapshot?.summary.totalCount ?? rewardRoles.length;
  const inviteUrl = snapshot?.discord.inviteUrl ?? DISCORD_INVITE_URL;
  const progressWidth = total > 0 ? (unlocked / total) * 100 : 0;

  return (
    <div className="min-h-full px-4 pt-6 pb-8">
      <div className="max-w-3xl mx-auto space-y-4">
        <ReferralRewardsSection />
        <RewardsCouponSection />

        <div className="bg-white rounded-2xl shadow-soft border border-stone-200 p-6">
          <div className="flex flex-col gap-5 md:flex-row md:items-center md:justify-between">
            <div className="space-y-2">
              <div className="inline-flex items-center gap-2 rounded-full border border-amber-200 bg-amber-50 px-3 py-1 text-xs font-medium text-amber-700">
                Discord Rewards
              </div>
              <h1 className="text-3xl font-semibold text-stone-900">Earn community roles</h1>
              <p className="max-w-xl text-sm text-stone-600">
                Join the OpenHuman Discord, connect your account, and track backend-synced rewards
                and role assignments from one place.
              </p>
            </div>

            <div className="flex flex-col gap-2 sm:flex-row">
              <button
                onClick={() => window.open(inviteUrl, '_blank', 'noopener,noreferrer')}
                className="inline-flex items-center justify-center gap-2 rounded-xl bg-stone-900 px-4 py-3 text-sm font-medium text-white transition-colors hover:bg-stone-800">
                <svg className="w-4 h-4" fill="currentColor" viewBox="0 0 24 24" aria-hidden="true">
                  <path d="M20.317 4.369A19.79 19.79 0 0 0 15.885 3c-.191.328-.403.775-.552 1.124a18.27 18.27 0 0 0-5.29 0A11.56 11.56 0 0 0 9.49 3a19.74 19.74 0 0 0-4.433 1.369C2.253 8.51 1.492 12.55 1.872 16.533a19.9 19.9 0 0 0 5.239 2.673c.423-.58.8-1.196 1.123-1.845a12.84 12.84 0 0 1-1.767-.85c.148-.106.292-.217.43-.332c3.408 1.6 7.104 1.6 10.472 0c.14.115.283.226.43.332c-.565.338-1.157.623-1.771.851c.322.648.698 1.264 1.123 1.844a19.84 19.84 0 0 0 5.241-2.673c.446-4.617-.761-8.621-3.787-12.164ZM9.46 14.088c-1.02 0-1.855-.936-1.855-2.084c0-1.148.82-2.084 1.855-2.084c1.044 0 1.87.944 1.855 2.084c0 1.148-.82 2.084-1.855 2.084Zm5.08 0c-1.02 0-1.855-.936-1.855-2.084c0-1.148.82-2.084 1.855-2.084c1.044 0 1.87.944 1.855 2.084c0 1.148-.812 2.084-1.855 2.084Z" />
                </svg>
                Join Discord
              </button>
              <button
                onClick={() => navigate('/settings/messaging')}
                className="inline-flex items-center justify-center rounded-xl border border-stone-200 bg-white px-4 py-3 text-sm font-medium text-stone-700 transition-colors hover:bg-stone-50">
                Connect Discord
              </button>
            </div>
          </div>
        </div>

        {error ? (
          <div
            role="alert"
            className="rounded-2xl border border-amber-200 bg-amber-50 px-4 py-3 text-sm text-amber-800">
            Rewards sync is unavailable right now. The page is showing connection guidance without
            claiming new unlocks. Details: {error}
          </div>
        ) : null}

        <div className="grid gap-4 md:grid-cols-[1.1fr_1.9fr]">
          <div className="bg-white rounded-2xl shadow-soft border border-stone-200 p-5">
            <div className="text-xs font-medium uppercase tracking-[0.18em] text-stone-400">
              Progress
            </div>
            <div className="mt-3 text-3xl font-semibold text-stone-900">
              {isLoading ? '...' : `${unlocked}/${total}`}
            </div>
            <p className="mt-2 text-sm text-stone-600">
              {snapshot
                ? 'Server-tracked achievements and Discord reward state.'
                : isLoading
                  ? 'Loading rewards from the backend.'
                  : 'Waiting for backend rewards data.'}
            </p>

            <div className="mt-5 h-2 overflow-hidden rounded-full bg-stone-100">
              <div
                className="h-full rounded-full bg-gradient-to-r from-primary-500 to-amber-400 transition-all duration-300"
                style={{ width: `${progressWidth}%` }}
              />
            </div>

            <div className="mt-5 space-y-3 text-sm text-stone-600">
              <div className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
                <span>Discord linked</span>
                <span className={snapshot?.discord.linked ? 'text-sage-600' : 'text-stone-500'}>
                  {snapshot ? (snapshot.discord.linked ? 'Yes' : 'No') : 'Unknown'}
                </span>
              </div>
              <div className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
                <span>Discord server</span>
                <span className="text-stone-900">{discordMembershipLabel(snapshot)}</span>
              </div>
              <div className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
                <span>Plan</span>
                <span className="text-stone-900">
                  {snapshot?.summary.plan ?? user?.subscription?.plan ?? 'FREE'}
                </span>
              </div>
              <div className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
                <span>Current streak</span>
                <span className="text-stone-900">
                  {snapshot ? `${snapshot.metrics.currentStreakDays} days` : 'Unknown'}
                </span>
              </div>
              <div className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
                <span>Cumulative tokens</span>
                <span className="text-stone-900">
                  {snapshot ? formatNumber(snapshot.metrics.cumulativeTokens) : 'Unknown'}
                </span>
              </div>
            </div>
          </div>

          <div className="space-y-3">
            {isLoading ? (
              <div className="rounded-2xl border border-stone-200 bg-white p-5 shadow-soft">
                <div className="text-sm text-stone-600">Loading rewards…</div>
              </div>
            ) : rewardRoles.length > 0 ? (
              rewardRoles.map(role => (
                <div
                  key={role.id}
                  className={`rounded-2xl border p-5 shadow-soft ${
                    role.unlocked ? 'border-sage-200 bg-white' : 'border-stone-200 bg-white/90'
                  }`}>
                  <div className="flex items-start justify-between gap-4">
                    <div className="space-y-1">
                      <div className="flex items-center gap-2">
                        <h2 className="text-lg font-semibold text-stone-900">{role.title}</h2>
                        <span
                          className={`rounded-full px-2.5 py-1 text-[11px] font-medium ${
                            role.unlocked
                              ? 'bg-sage-100 text-sage-700'
                              : 'bg-stone-100 text-stone-500'
                          }`}>
                          {role.unlocked ? 'Unlocked' : 'Locked'}
                        </span>
                      </div>
                      <p className="text-sm text-stone-600">{role.description}</p>
                    </div>

                    <div
                      className={`flex h-10 w-10 items-center justify-center rounded-xl ${
                        role.unlocked ? 'bg-sage-100 text-sage-700' : 'bg-stone-100 text-stone-500'
                      }`}>
                      {role.unlocked ? (
                        <svg
                          className="w-5 h-5"
                          fill="none"
                          stroke="currentColor"
                          viewBox="0 0 24 24">
                          <path
                            strokeLinecap="round"
                            strokeLinejoin="round"
                            strokeWidth={2}
                            d="M5 13l4 4L19 7"
                          />
                        </svg>
                      ) : (
                        <svg
                          className="w-5 h-5"
                          fill="none"
                          stroke="currentColor"
                          viewBox="0 0 24 24">
                          <path
                            strokeLinecap="round"
                            strokeLinejoin="round"
                            strokeWidth={2}
                            d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"
                          />
                        </svg>
                      )}
                    </div>
                  </div>

                  <div className="mt-4 grid gap-2 rounded-xl border border-stone-200 bg-stone-50 px-3 py-3 sm:grid-cols-[1.3fr_1fr]">
                    <div>
                      <div className="text-xs font-medium uppercase tracking-wide text-stone-400">
                        Unlock Action
                      </div>
                      <div className="mt-1 text-sm text-stone-800">{role.actionLabel}</div>
                    </div>
                    <div>
                      <div className="text-xs font-medium uppercase tracking-wide text-stone-400">
                        Server Progress
                      </div>
                      <div className="mt-1 text-sm font-medium text-stone-600">
                        {role.progressLabel}
                      </div>
                    </div>
                    <div>
                      <div className="text-xs font-medium uppercase tracking-wide text-stone-400">
                        Discord Role
                      </div>
                      <div className="mt-1 text-sm text-stone-800">
                        {roleStatusLabel(role.discordRoleStatus)}
                      </div>
                    </div>
                    <div>
                      <div className="text-xs font-medium uppercase tracking-wide text-stone-400">
                        Credit Reward
                      </div>
                      <div className="mt-1 text-sm text-stone-800">
                        {role.creditAmountUsd != null ? `$${role.creditAmountUsd}` : 'None'}
                      </div>
                    </div>
                  </div>
                </div>
              ))
            ) : (
              <div className="rounded-2xl border border-stone-200 bg-white p-5 shadow-soft">
                <h2 className="text-lg font-semibold text-stone-900">Rewards sync pending</h2>
                <p className="mt-2 text-sm text-stone-600">
                  The backend did not return achievement data yet. Join Discord and connect your
                  account now, then refresh this page once sync is available again.
                </p>
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
};

export default Rewards;
