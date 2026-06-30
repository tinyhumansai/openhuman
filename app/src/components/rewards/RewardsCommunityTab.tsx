import createDebug from 'debug';
import { useCallback, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { rewardsApi } from '../../services/api/rewardsApi';
import { callCoreRpc } from '../../services/coreRpcClient';
import type { RewardsAchievement, RewardsSnapshot } from '../../types/rewards';
import { DISCORD_INVITE_URL } from '../../utils/links';
import { setOAuthReturnRoute } from '../../utils/oauthReturnRoute';
import { openUrl } from '../../utils/openUrl';
import Button from '../ui/Button';

const log = createDebug('rewards:discord');

// discordMembershipLabel is now inlined into JSX to access t()

function formatNumber(value: number): string {
  return new Intl.NumberFormat('en-US').format(Math.max(0, Math.trunc(value)));
}

function roleAccentTone(index: number) {
  const tones = [
    {
      iconBg: 'bg-amber-50 dark:bg-amber-500/10',
      iconText: 'text-amber-600 dark:text-amber-300',
      iconBorder: 'border-amber-100 dark:border-amber-500/20',
    },
    {
      iconBg: 'bg-blue-50 dark:bg-blue-500/10',
      iconText: 'text-primary-600 dark:text-primary-300',
      iconBorder: 'border-blue-100 dark:border-blue-500/20',
    },
    {
      iconBg: 'bg-slate-100 dark:bg-slate-500/10',
      iconText: 'text-slate-600 dark:text-slate-300',
      iconBorder: 'border-slate-200 dark:border-slate-500/20',
    },
    {
      iconBg: 'bg-emerald-50 dark:bg-emerald-500/10',
      iconText: 'text-emerald-600 dark:text-emerald-300',
      iconBorder: 'border-emerald-100 dark:border-emerald-500/20',
    },
  ] as const;

  return tones[index % tones.length];
}

function roleGlyph(index: number) {
  switch (index % 4) {
    case 0:
      return (
        <path
          d="M12 3l2.4 4.86 5.36.78-3.88 3.78.92 5.35L12 15.27 7.2 17.77l.92-5.35L4.24 8.64l5.36-.78L12 3Z"
          fill="currentColor"
        />
      );
    case 1:
      return (
        <path
          d="M12 2.5 14.78 8l5.97.87-4.32 4.2 1.02 5.93L12 16.2 6.55 19l1.04-5.93-4.33-4.2L9.22 8 12 2.5Z"
          fill="currentColor"
        />
      );
    case 2:
      return (
        <path
          d="M12 3 5 6v5c0 4.08 2.87 7.9 7 8.9 4.13-1 7-4.82 7-8.9V6l-7-3Z"
          fill="currentColor"
        />
      );
    default:
      return (
        <path
          d="M12 2a5 5 0 0 1 5 5v3h1a2 2 0 0 1 2 2v2c0 4.42-3.58 8-8 8s-8-3.58-8-8v-2a2 2 0 0 1 2-2h1V7a5 5 0 0 1 5-5Zm-3 8h6V7a3 3 0 1 0-6 0v3Z"
          fill="currentColor"
        />
      );
  }
}

interface RewardsCommunityTabProps {
  error: string | null;
  isLoading: boolean;
  onRetry?: () => void;
  snapshot: RewardsSnapshot | null;
}

export default function RewardsCommunityTab({
  error,
  isLoading,
  onRetry,
  snapshot,
}: RewardsCommunityTabProps) {
  const { t } = useT();
  const [connectState, setConnectState] = useState<'idle' | 'connecting' | 'error'>('idle');
  const [disconnectState, setDisconnectState] = useState<'idle' | 'disconnecting' | 'error'>(
    'idle'
  );
  const rewardRoles: RewardsAchievement[] = snapshot?.achievements ?? [];
  const unlocked =
    snapshot?.summary.unlockedCount ?? rewardRoles.filter(role => role.unlocked).length;
  const total = snapshot?.summary.totalCount ?? rewardRoles.length;
  const inviteUrl = snapshot?.discord.inviteUrl ?? DISCORD_INVITE_URL;
  const progressPercent = total > 0 ? Math.round((unlocked / total) * 100) : 0;
  const achievementSlots =
    rewardRoles.length > 0 ? rewardRoles.slice(0, 8) : new Array(4).fill(null);
  const ringCircumference = 2 * Math.PI * 24;
  const ringOffset = ringCircumference - (progressPercent / 100) * ringCircumference;
  const discordLinked = snapshot?.discord.linked ?? false;
  const discordUsername = snapshot?.discord.username ?? null;
  const membershipStatus = snapshot?.discord.membershipStatus ?? null;
  const assignedRoleCount = snapshot?.summary.assignedDiscordRoleCount ?? 0;
  // A connected member who unlocked a role-bearing achievement but has not joined the
  // server yet cannot receive the role — surface an actionable prompt to join.
  const hasUnlockedConfiguredRole = rewardRoles.some(role => role.unlocked && Boolean(role.roleId));
  const showClaimBanner =
    discordLinked && membershipStatus === 'not_in_guild' && hasUnlockedConfiguredRole;

  const handleConnectDiscord = useCallback(async () => {
    log('connect discord requested');
    setConnectState('connecting');
    try {
      const response = await callCoreRpc<{ result: { oauthUrl?: string } }>({
        method: 'openhuman.auth.oauth_connect',
        params: { provider: 'discord' },
      });
      const oauthUrl = response.result?.oauthUrl;
      if (!oauthUrl) {
        throw new Error('missing oauthUrl in oauth_connect response');
      }
      log('opening discord oauth consent url');
      await openUrl(oauthUrl);
      // Persist the return route only after the consent URL actually launched, so a failed
      // initiation never leaves a stale route that could misroute a later OAuth success.
      setOAuthReturnRoute('/rewards');
      // Reset so the button is usable again if the user cancels; once the snapshot
      // refetches with discord.linked the connected state takes over.
      setConnectState('idle');
    } catch (err) {
      log('connect discord failed error=%s', err instanceof Error ? err.message : String(err));
      setConnectState('error');
    }
  }, []);

  const handleDisconnectDiscord = useCallback(async () => {
    log('disconnect discord requested');
    setDisconnectState('disconnecting');
    try {
      // Clears user.discordId/discordUsername on the backend (idempotent), which flips the
      // rewards snapshot back to unlinked.
      await rewardsApi.disconnectDiscord();
      log('disconnect discord ok; refreshing snapshot');
      setDisconnectState('idle');
      // Refetch the snapshot so the connected state flips back to the Connect button (re-link path).
      onRetry?.();
    } catch (err) {
      log('disconnect discord failed error=%s', err instanceof Error ? err.message : String(err));
      setDisconnectState('error');
    }
  }, [onRetry]);
  return (
    <>
      <section className="relative overflow-hidden rounded-[1.25rem] bg-gradient-to-br from-[#004ad0] to-[#2b64f1] p-6 text-white shadow-[0_20px_40px_rgba(25,28,30,0.08)]">
        <div className="relative z-10 space-y-4">
          <div className="space-y-2">
            <h1 className="text-2xl font-bold tracking-tight text-white">
              {t('rewards.community.heroTitle')}
            </h1>
            <p className="text-sm font-medium leading-relaxed text-white/90">
              {t('rewards.community.heroSubtitle')}
            </p>
          </div>
          <div className="flex flex-col gap-2 sm:flex-row">
            {discordLinked ? (
              <>
                <div
                  data-testid="rewards-discord-connected"
                  className="inline-flex items-center justify-center gap-2 rounded-xl bg-surface/15 px-4 py-3 text-sm font-semibold text-white">
                  <svg
                    className="h-4 w-4"
                    viewBox="0 0 24 24"
                    fill="currentColor"
                    aria-hidden="true">
                    <path d="M9 16.17 4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z" />
                  </svg>
                  {discordUsername
                    ? t('rewards.community.discordConnectedAs').replace(
                        '{username}',
                        discordUsername
                      )
                    : t('rewards.community.discordConnected')}
                </div>
                <button
                  onClick={() => {
                    void handleDisconnectDiscord();
                  }}
                  disabled={disconnectState === 'disconnecting'}
                  data-testid="rewards-disconnect-discord"
                  className="inline-flex items-center justify-center gap-2 rounded-xl border border-white/20 bg-surface/10 px-4 py-3 text-sm font-semibold text-white backdrop-blur-sm transition-colors hover:bg-white/15 disabled:cursor-not-allowed disabled:opacity-70">
                  {disconnectState === 'disconnecting'
                    ? t('rewards.community.disconnectingDiscord')
                    : t('rewards.community.disconnectDiscord')}
                </button>
              </>
            ) : (
              <button
                onClick={() => {
                  void handleConnectDiscord();
                }}
                disabled={connectState === 'connecting'}
                data-testid="rewards-connect-discord"
                className="inline-flex items-center justify-center gap-2 rounded-xl bg-surface px-4 py-3 text-sm font-semibold text-primary-700 dark:text-primary-300 shadow-lg transition-transform active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-70">
                <svg
                  className="w-4 h-4"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                  aria-hidden="true">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M13.828 10.172a4 4 0 0 0-5.656 0l-1 1a4 4 0 0 0 5.656 5.656l.586-.586m-3.242-2.828a4 4 0 0 0 5.656 0l1-1a4 4 0 1 0-5.656-5.656l-.586.586"
                  />
                </svg>
                {connectState === 'connecting'
                  ? t('rewards.community.connectingDiscord')
                  : t('rewards.community.connectDiscord')}
              </button>
            )}
            <button
              onClick={() => {
                void openUrl(inviteUrl);
              }}
              className="inline-flex items-center justify-center gap-2 rounded-xl border border-white/20 bg-surface/10 px-4 py-3 text-sm font-semibold text-white backdrop-blur-sm transition-colors hover:bg-white/15">
              <svg className="h-4 w-4" fill="currentColor" viewBox="0 0 24 24" aria-hidden="true">
                <path d="M20.317 4.369A19.79 19.79 0 0 0 15.885 3c-.191.328-.403.775-.552 1.124a18.27 18.27 0 0 0-5.29 0A11.56 11.56 0 0 0 9.49 3a19.74 19.74 0 0 0-4.433 1.369C2.253 8.51 1.492 12.55 1.872 16.533a19.9 19.9 0 0 0 5.239 2.673c.423-.58.8-1.196 1.123-1.845a12.84 12.84 0 0 1-1.767-.85c.148-.106.292-.217.43-.332c3.408 1.6 7.104 1.6 10.472 0c.14.115.283.226.43.332c-.565.338-1.157.623-1.771.851c.322.648.698 1.264 1.123 1.844a19.84 19.84 0 0 0 5.241-2.673c.446-4.617-.761-8.621-3.787-12.164ZM9.46 14.088c-1.02 0-1.855-.936-1.855-2.084c0-1.148.82-2.084 1.855-2.084c1.044 0 1.87.944 1.855 2.084c0 1.148-.82 2.084-1.855 2.084Zm5.08 0c-1.02 0-1.855-.936-1.855-2.084c0-1.148.82-2.084 1.855-2.084c1.044 0 1.87.944 1.855 2.084c0 1.148-.812 2.084-1.855 2.084Z" />
              </svg>
              {t('rewards.community.joinDiscord')}
            </button>
          </div>
          {connectState === 'error' ? (
            <p
              role="alert"
              data-testid="rewards-connect-discord-error"
              className="text-xs font-medium text-white/90">
              {t('rewards.community.connectDiscordError')}
            </p>
          ) : null}
          {discordLinked && disconnectState === 'error' ? (
            <p
              role="alert"
              data-testid="rewards-disconnect-discord-error"
              className="text-xs font-medium text-white/90">
              {t('rewards.community.disconnectDiscordError')}
            </p>
          ) : null}
        </div>
        <div className="absolute -right-10 -top-10 h-32 w-32 rounded-full bg-surface/10 blur-2xl" />
        <div className="absolute -bottom-10 -left-8 h-24 w-24 rounded-full bg-surface/15 blur-xl" />
      </section>

      {error ? (
        <div
          role="alert"
          data-testid="rewards-error"
          className="flex flex-wrap items-center justify-between gap-3 rounded-2xl border border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/10 px-4 py-3 text-sm text-amber-800 dark:text-amber-200">
          <span>
            {t('rewards.community.syncUnavailable')} {error}
          </span>
          {onRetry ? (
            <button
              type="button"
              data-testid="rewards-retry"
              onClick={onRetry}
              disabled={isLoading}
              className="rounded-full border border-amber-300 dark:border-amber-500/40 bg-surface px-3 py-1 text-xs font-semibold text-amber-800 dark:text-amber-200 shadow-sm transition-colors hover:bg-amber-100 dark:bg-amber-500/20 disabled:cursor-not-allowed disabled:opacity-60">
              {isLoading ? t('rewards.community.retrying') : t('rewards.community.tryAgain')}
            </button>
          ) : null}
        </div>
      ) : null}

      <div className="space-y-4">
        <section className="rounded-[1.25rem] bg-surface p-6 shadow-[0_4px_20px_rgba(25,28,30,0.04)]">
          <div className="mb-6 flex items-center justify-between gap-4">
            <div>
              <h2 className="text-lg font-bold text-content">
                {t('rewards.community.yourProgress')}
              </h2>
              <p className="text-xs text-content-muted">
                {isLoading
                  ? t('rewards.community.loadingRewards')
                  : t('rewards.community.achievementsUnlocked')
                      .replace('{unlocked}', String(unlocked))
                      .replace('{total}', String(total))}
              </p>
            </div>
            <div className="relative flex h-14 w-14 items-center justify-center">
              <svg className="h-full w-full -rotate-90" viewBox="0 0 56 56" aria-hidden="true">
                <circle
                  cx="28"
                  cy="28"
                  r="24"
                  fill="transparent"
                  stroke="currentColor"
                  strokeWidth="4"
                  className="text-stone-200"
                />
                <circle
                  cx="28"
                  cy="28"
                  r="24"
                  fill="transparent"
                  stroke="currentColor"
                  strokeWidth="4"
                  strokeDasharray={ringCircumference}
                  strokeDashoffset={ringOffset}
                  className="text-primary-600 dark:text-primary-300 transition-all duration-300"
                />
              </svg>
              <span className="absolute text-sm font-bold text-content">{progressPercent}%</span>
            </div>
          </div>

          <div className="flex gap-4 overflow-x-auto pb-1 scrollbar-hide">
            {achievementSlots.map((role, index) => (
              <div
                key={role?.id ?? `placeholder-${index}`}
                className={`flex h-16 w-16 flex-shrink-0 items-center justify-center rounded-full border-2 ${
                  role?.unlocked
                    ? 'border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 text-primary-600 dark:text-primary-300'
                    : 'border-dashed border-line-strong bg-surface-subtle text-content-faint'
                }`}>
                <svg className="h-6 w-6" viewBox="0 0 24 24" aria-hidden="true">
                  {roleGlyph(index)}
                </svg>
              </div>
            ))}
          </div>
        </section>

        <section className="space-y-3">
          <div className="flex items-center justify-between">
            <h2 className="text-lg font-bold text-content">
              {t('rewards.community.rolesAndRewards')}
            </h2>
          </div>
          {showClaimBanner ? (
            <div
              role="status"
              data-testid="rewards-claim-roles-banner"
              className="flex flex-wrap items-center justify-between gap-3 rounded-2xl border border-blue-100 dark:border-blue-500/30 bg-blue-50 dark:bg-blue-500/10 px-4 py-3">
              <div className="min-w-0">
                <p className="text-sm font-bold text-content">
                  {t('rewards.community.roleClaimTitle')}
                </p>
                <p className="mt-0.5 text-xs leading-relaxed text-content-secondary">
                  {t('rewards.community.roleClaimDesc')}
                </p>
              </div>
              <Button
                variant="primary"
                size="md"
                data-testid="rewards-claim-roles-join"
                onClick={() => {
                  void openUrl(inviteUrl);
                }}
                className="flex-shrink-0">
                {t('rewards.community.joinDiscord')}
              </Button>
            </div>
          ) : null}
          {isLoading ? (
            <div className="rounded-2xl border border-line bg-surface p-5 shadow-soft">
              <div className="text-sm text-content-secondary">
                {t('rewards.community.loadingRewards')}
              </div>
            </div>
          ) : rewardRoles.length > 0 ? (
            rewardRoles.map((role, index) => {
              const tone = roleAccentTone(index);
              // Surface Discord role-assignment status only for a linked user's unlocked
              // achievements — locked badges have no role to claim yet.
              const roleStatus =
                discordLinked && role.unlocked
                  ? role.discordRoleStatus === 'assigned'
                    ? {
                        label: t('rewards.community.roleAssigned'),
                        classes:
                          'bg-emerald-50 text-emerald-700 dark:bg-emerald-500/10 dark:text-emerald-300',
                      }
                    : role.discordRoleStatus === 'not_assigned'
                      ? {
                          label: t('rewards.community.rolePending'),
                          classes:
                            'bg-amber-50 text-amber-700 dark:bg-amber-500/10 dark:text-amber-300',
                        }
                      : role.discordRoleStatus === 'not_in_guild'
                        ? {
                            label: t('rewards.community.roleJoinToClaim'),
                            classes:
                              'bg-blue-50 text-primary-700 dark:bg-blue-500/10 dark:text-primary-300',
                          }
                        : null
                  : null;

              return (
                <div
                  key={role.id}
                  className={`rounded-[1.25rem] bg-surface p-5 shadow-sm transition-shadow hover:shadow-md ${
                    role.unlocked
                      ? 'ring-1 ring-primary-100 dark:ring-primary-500/20'
                      : 'ring-1 ring-black/[0.04] dark:ring-white/[0.06]'
                  }`}>
                  <div className="flex items-start justify-between gap-4">
                    <div className="flex gap-4">
                      <div
                        className={`flex h-12 w-12 flex-shrink-0 items-center justify-center rounded-xl border ${tone.iconBg} ${tone.iconText} ${tone.iconBorder}`}>
                        <svg className="h-6 w-6" viewBox="0 0 24 24" aria-hidden="true">
                          {roleGlyph(index)}
                        </svg>
                      </div>
                      <div>
                        <h3 className="text-base font-bold text-content">{role.title}</h3>
                        <p className="mt-1 text-xs leading-relaxed text-content-secondary">
                          {role.description}
                        </p>
                      </div>
                    </div>
                    <div className="flex items-center gap-1 text-primary-700 dark:text-primary-300">
                      <span className="text-[10px] font-bold uppercase tracking-[0.16em]">
                        {role.unlocked
                          ? t('rewards.community.unlocked')
                          : t('rewards.community.locked')}
                      </span>
                      <svg
                        className="h-4 w-4"
                        viewBox="0 0 24 24"
                        fill="currentColor"
                        aria-hidden="true">
                        {role.unlocked ? (
                          <path d="M9 16.17 4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z" />
                        ) : (
                          <path d="M12 2a5 5 0 0 1 5 5v3h1a2 2 0 0 1 2 2v2c0 4.42-3.58 8-8 8s-8-3.58-8-8v-2a2 2 0 0 1 2-2h1V7a5 5 0 0 1 5-5Zm-3 8h6V7a3 3 0 1 0-6 0v3Z" />
                        )}
                      </svg>
                    </div>
                  </div>
                  {roleStatus ? (
                    <div className="mt-3">
                      <span
                        data-testid={`rewards-role-status-${role.id}`}
                        className={`inline-flex items-center rounded-full px-2.5 py-1 text-[11px] font-semibold ${roleStatus.classes}`}>
                        {roleStatus.label}
                      </span>
                    </div>
                  ) : null}
                </div>
              );
            })
          ) : (
            <div className="rounded-2xl border border-line bg-surface p-5 shadow-soft">
              <h2 className="text-lg font-semibold text-content">
                {t('rewards.community.syncPending')}
              </h2>
              <p className="mt-2 text-sm text-content-secondary">
                {t('rewards.community.syncPendingDesc')}
              </p>
            </div>
          )}
        </section>

        <section className="rounded-[1.25rem] bg-[#f2f4f6] dark:bg-surface-muted/60 p-4 text-sm text-content-secondary">
          <div className="flex items-center justify-between gap-3">
            <span>{t('rewards.community.discordServer')}</span>
            <span className="font-semibold text-content">
              {!snapshot
                ? t('rewards.community.discordWaiting')
                : snapshot.discord.membershipStatus === 'member'
                  ? t('rewards.community.discordMember')
                  : snapshot.discord.membershipStatus === 'not_in_guild'
                    ? t('rewards.community.discordLinkedNotInGuild')
                    : snapshot.discord.membershipStatus === 'not_linked'
                      ? t('rewards.community.discordNotLinked')
                      : t('rewards.community.discordStatusUnavailable')}
            </span>
          </div>
          {discordLinked && discordUsername ? (
            <div className="mt-3 flex items-center justify-between gap-3">
              <span>{t('rewards.community.discordAccount')}</span>
              <span data-testid="rewards-discord-username" className="font-semibold text-content">
                {discordUsername}
              </span>
            </div>
          ) : null}
          {discordLinked && membershipStatus === 'member' ? (
            <div className="mt-3 flex items-center justify-between gap-3">
              <span>{t('rewards.community.rolesAndRewards')}</span>
              <span data-testid="rewards-roles-assigned" className="font-semibold text-content">
                {t('rewards.community.roleAssignmentCount')
                  .replace('{assigned}', String(assignedRoleCount))
                  .replace('{unlocked}', String(unlocked))}
              </span>
            </div>
          ) : null}
          <div className="mt-3 flex items-center justify-between gap-3">
            <span>{t('rewards.community.currentStreak')}</span>
            <span className="font-semibold text-content">
              {snapshot
                ? t('rewards.community.streakDays').replace(
                    '{n}',
                    String(snapshot.metrics.currentStreakDays)
                  )
                : t('rewards.community.unknown')}
            </span>
          </div>
          <div className="mt-3 flex items-center justify-between gap-3">
            <span>{t('rewards.community.cumulativeTokens')}</span>
            <span className="font-semibold text-content">
              {snapshot
                ? formatNumber(snapshot.metrics.cumulativeTokens)
                : t('rewards.community.unknown')}
            </span>
          </div>
        </section>
      </div>
    </>
  );
}
