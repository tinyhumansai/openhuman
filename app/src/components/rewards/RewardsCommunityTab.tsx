import { useNavigate } from 'react-router-dom';

import type { RewardsAchievement, RewardsSnapshot } from '../../types/rewards';
import { DISCORD_INVITE_URL } from '../../utils/links';

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

function formatNumber(value: number): string {
  return new Intl.NumberFormat('en-US').format(Math.max(0, Math.trunc(value)));
}

function roleAccentTone(index: number) {
  const tones = [
    { iconBg: 'bg-amber-50', iconText: 'text-amber-600', iconBorder: 'border-amber-100' },
    { iconBg: 'bg-blue-50', iconText: 'text-primary-600', iconBorder: 'border-blue-100' },
    { iconBg: 'bg-slate-100', iconText: 'text-slate-600', iconBorder: 'border-slate-200' },
    { iconBg: 'bg-emerald-50', iconText: 'text-emerald-600', iconBorder: 'border-emerald-100' },
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
  snapshot: RewardsSnapshot | null;
}

export default function RewardsCommunityTab({
  error,
  isLoading,
  snapshot,
}: RewardsCommunityTabProps) {
  const navigate = useNavigate();
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
  return (
    <>
      <section className="relative overflow-hidden rounded-[1.25rem] bg-gradient-to-br from-[#004ad0] to-[#2b64f1] p-6 text-white shadow-[0_20px_40px_rgba(25,28,30,0.08)]">
        <div className="relative z-10 space-y-4">
          <div className="space-y-2">
            <h1 className="text-2xl font-bold tracking-tight text-white">
              Earn Rewards & Discord Roles
            </h1>
            <p className="text-sm font-medium leading-relaxed text-white/90">
              Unlock exclusive channels, supporter badges, and backend-synced rewards by connecting
              your Discord account.
            </p>
          </div>
          <div className="flex flex-col gap-2 sm:flex-row">
            <button
              onClick={() => navigate('/settings/messaging')}
              className="inline-flex items-center justify-center gap-2 rounded-xl bg-white px-4 py-3 text-sm font-semibold text-primary-700 shadow-lg transition-transform active:scale-[0.98]">
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
              Connect Discord
            </button>
            <button
              onClick={() => window.open(inviteUrl, '_blank', 'noopener,noreferrer')}
              className="inline-flex items-center justify-center gap-2 rounded-xl border border-white/20 bg-white/10 px-4 py-3 text-sm font-semibold text-white backdrop-blur-sm transition-colors hover:bg-white/15">
              <svg className="h-4 w-4" fill="currentColor" viewBox="0 0 24 24" aria-hidden="true">
                <path d="M20.317 4.369A19.79 19.79 0 0 0 15.885 3c-.191.328-.403.775-.552 1.124a18.27 18.27 0 0 0-5.29 0A11.56 11.56 0 0 0 9.49 3a19.74 19.74 0 0 0-4.433 1.369C2.253 8.51 1.492 12.55 1.872 16.533a19.9 19.9 0 0 0 5.239 2.673c.423-.58.8-1.196 1.123-1.845a12.84 12.84 0 0 1-1.767-.85c.148-.106.292-.217.43-.332c3.408 1.6 7.104 1.6 10.472 0c.14.115.283.226.43.332c-.565.338-1.157.623-1.771.851c.322.648.698 1.264 1.123 1.844a19.84 19.84 0 0 0 5.241-2.673c.446-4.617-.761-8.621-3.787-12.164ZM9.46 14.088c-1.02 0-1.855-.936-1.855-2.084c0-1.148.82-2.084 1.855-2.084c1.044 0 1.87.944 1.855 2.084c0 1.148-.82 2.084-1.855 2.084Zm5.08 0c-1.02 0-1.855-.936-1.855-2.084c0-1.148.82-2.084 1.855-2.084c1.044 0 1.87.944 1.855 2.084c0 1.148-.812 2.084-1.855 2.084Z" />
              </svg>
              Join Discord
            </button>
          </div>
        </div>
        <div className="absolute -right-10 -top-10 h-32 w-32 rounded-full bg-white/10 blur-2xl" />
        <div className="absolute -bottom-10 -left-8 h-24 w-24 rounded-full bg-white/15 blur-xl" />
      </section>

      {error ? (
        <div
          role="alert"
          className="rounded-2xl border border-amber-200 bg-amber-50 px-4 py-3 text-sm text-amber-800">
          Rewards sync is unavailable right now. The page is showing connection guidance without
          claiming new unlocks. Details: {error}
        </div>
      ) : null}

      <div className="space-y-4">
        <section className="rounded-[1.25rem] bg-white p-6 shadow-[0_4px_20px_rgba(25,28,30,0.04)]">
          <div className="mb-6 flex items-center justify-between gap-4">
            <div>
              <h2 className="text-lg font-bold text-stone-900">Your Progress</h2>
              <p className="text-xs text-stone-500">
                {isLoading ? 'Loading rewards…' : `${unlocked} of ${total} achievements unlocked`}
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
                  className="text-primary-600 transition-all duration-300"
                />
              </svg>
              <span className="absolute text-sm font-bold text-stone-900">{progressPercent}%</span>
            </div>
          </div>

          <div className="flex gap-4 overflow-x-auto pb-1 scrollbar-hide">
            {achievementSlots.map((role, index) => (
              <div
                key={role?.id ?? `placeholder-${index}`}
                className={`flex h-16 w-16 flex-shrink-0 items-center justify-center rounded-full border-2 ${
                  role?.unlocked
                    ? 'border-primary-200 bg-primary-50 text-primary-600'
                    : 'border-dashed border-stone-300 bg-stone-100 text-stone-400'
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
            <h2 className="text-lg font-bold text-stone-900">Roles & Rewards</h2>
          </div>
          {isLoading ? (
            <div className="rounded-2xl border border-stone-200 bg-white p-5 shadow-soft">
              <div className="text-sm text-stone-600">Loading rewards…</div>
            </div>
          ) : rewardRoles.length > 0 ? (
            rewardRoles.map((role, index) => {
              const tone = roleAccentTone(index);

              return (
                <div
                  key={role.id}
                  className={`rounded-[1.25rem] bg-white p-5 shadow-sm transition-shadow hover:shadow-md ${
                    role.unlocked ? 'ring-1 ring-primary-100' : 'ring-1 ring-black/[0.04]'
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
                        <h3 className="text-base font-bold text-stone-900">{role.title}</h3>
                        <p className="mt-1 text-xs leading-relaxed text-stone-600">
                          {role.description}
                        </p>
                      </div>
                    </div>
                    <div className="flex items-center gap-1 text-primary-700">
                      <span className="text-[10px] font-bold uppercase tracking-[0.16em]">
                        {role.unlocked ? 'Unlocked' : 'Locked'}
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
                </div>
              );
            })
          ) : (
            <div className="rounded-2xl border border-stone-200 bg-white p-5 shadow-soft">
              <h2 className="text-lg font-semibold text-stone-900">Rewards sync pending</h2>
              <p className="mt-2 text-sm text-stone-600">
                The backend did not return achievement data yet. Join Discord and connect your
                account now, then refresh this page once sync is available again.
              </p>
            </div>
          )}
        </section>

        <section className="rounded-[1.25rem] bg-[#f2f4f6] p-4 text-sm text-stone-600">
          <div className="flex items-center justify-between gap-3">
            <span>Discord server</span>
            <span className="font-semibold text-stone-900">{discordMembershipLabel(snapshot)}</span>
          </div>
          <div className="mt-3 flex items-center justify-between gap-3">
            <span>Current streak</span>
            <span className="font-semibold text-stone-900">
              {snapshot ? `${snapshot.metrics.currentStreakDays} days` : 'Unknown'}
            </span>
          </div>
          <div className="mt-3 flex items-center justify-between gap-3">
            <span>Cumulative tokens</span>
            <span className="font-semibold text-stone-900">
              {snapshot ? formatNumber(snapshot.metrics.cumulativeTokens) : 'Unknown'}
            </span>
          </div>
        </section>
      </div>
    </>
  );
}
