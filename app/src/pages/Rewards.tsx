import { useMemo } from 'react';
import { useNavigate } from 'react-router-dom';

import ReferralRewardsSection from '../components/referral/ReferralRewardsSection';
import { useUser } from '../hooks/useUser';
import { useAppSelector } from '../store/hooks';

const DISCORD_INVITE_URL = 'https://discord.com/invite/k23Kn8nK';

interface RewardRole {
  id: string;
  title: string;
  description: string;
  actionLabel: string;
  unlocked: boolean;
  progressLabel: string;
}

const Rewards = () => {
  const navigate = useNavigate();
  const { user } = useUser();
  const threads = useAppSelector(state => state.thread.threads);
  const channelConnections = useAppSelector(state => state.channelConnections.connections);

  const totalMessages = useAppSelector(state =>
    Object.values(state.thread.messagesByThreadId).reduce(
      (sum, messages) => sum + messages.length,
      0
    )
  );

  const hasDiscordConnection = useMemo(
    () =>
      Object.values(channelConnections.discord).some(
        connection => connection?.status === 'connected'
      ),
    [channelConnections.discord]
  );

  const rewardRoles: RewardRole[] = [
    {
      id: 'first-contact',
      title: 'First Contact',
      description: 'Send your first message to OpenHuman.',
      actionLabel: 'Start one chat',
      unlocked: totalMessages > 0,
      progressLabel: totalMessages > 0 ? 'Unlocked' : '0 / 1 messages sent',
    },
    {
      id: 'discord-pilot',
      title: 'Discord Pilot',
      description: 'Link Discord messaging so OpenHuman can reach you there.',
      actionLabel: 'Connect Discord in Messaging',
      unlocked: hasDiscordConnection,
      progressLabel: hasDiscordConnection ? 'Unlocked' : 'Discord not connected yet',
    },
    {
      id: 'power-user',
      title: 'Power User',
      description: 'Build momentum by actively using OpenHuman in multiple sessions.',
      actionLabel: 'Reach 10 total chat messages',
      unlocked: totalMessages >= 10,
      progressLabel: `${Math.min(totalMessages, 10)} / 10 messages`,
    },
    {
      id: 'supporter',
      title: 'Supporter',
      description: 'Unlock the supporter role with an active paid plan.',
      actionLabel: 'Upgrade to Basic or Pro',
      unlocked: !!user?.subscription?.hasActiveSubscription,
      progressLabel: user?.subscription?.hasActiveSubscription
        ? `${user.subscription.plan} plan active`
        : 'No active subscription',
    },
    {
      id: 'community-builder',
      title: 'Community Builder',
      description: 'Bring another human into the network through the invite system.',
      actionLabel: 'Redeem or participate in invites',
      unlocked: !!user?.referral?.invitedBy || threads.length > 1,
      progressLabel:
        !!user?.referral?.invitedBy || threads.length > 1
          ? 'Unlocked'
          : 'No invite activity detected yet',
    },
  ];

  const unlockedCount = rewardRoles.filter(role => role.unlocked).length;

  return (
    <div className="min-h-full px-4 pt-6 pb-8">
      <div className="max-w-3xl mx-auto space-y-4">
        <ReferralRewardsSection />

        <div className="bg-white rounded-2xl shadow-soft border border-stone-200 p-6">
          <div className="flex flex-col gap-5 md:flex-row md:items-center md:justify-between">
            <div className="space-y-2">
              <div className="inline-flex items-center gap-2 rounded-full border border-amber-200 bg-amber-50 px-3 py-1 text-xs font-medium text-amber-700">
                Discord Rewards
              </div>
              <h1 className="text-3xl font-semibold text-stone-900">Earn community roles</h1>
              <p className="max-w-xl text-sm text-stone-600">
                Join the OpenHuman Discord, connect your account, and unlock roles as you use more
                of the app.
              </p>
            </div>

            <div className="flex flex-col gap-2 sm:flex-row">
              <button
                onClick={() => window.open(DISCORD_INVITE_URL, '_blank', 'noopener,noreferrer')}
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

        <div className="grid gap-4 md:grid-cols-[1.1fr_1.9fr]">
          <div className="bg-white rounded-2xl shadow-soft border border-stone-200 p-5">
            <div className="text-xs font-medium uppercase tracking-[0.18em] text-stone-400">
              Progress
            </div>
            <div className="mt-3 text-3xl font-semibold text-stone-900">
              {unlockedCount}/{rewardRoles.length}
            </div>
            <p className="mt-2 text-sm text-stone-600">
              Roles unlocked from your current app activity.
            </p>

            <div className="mt-5 h-2 overflow-hidden rounded-full bg-stone-100">
              <div
                className="h-full rounded-full bg-gradient-to-r from-primary-500 to-amber-400 transition-all duration-300"
                style={{ width: `${(unlockedCount / rewardRoles.length) * 100}%` }}
              />
            </div>

            <div className="mt-5 space-y-3 text-sm text-stone-600">
              <div className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
                <span>Discord linked</span>
                <span className={hasDiscordConnection ? 'text-sage-600' : 'text-stone-500'}>
                  {hasDiscordConnection ? 'Yes' : 'No'}
                </span>
              </div>
              <div className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
                <span>Total messages</span>
                <span className="text-stone-900">{totalMessages}</span>
              </div>
              <div className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
                <span>Plan</span>
                <span className="text-stone-900">{user?.subscription?.plan ?? 'FREE'}</span>
              </div>
            </div>
          </div>

          <div className="space-y-3">
            {rewardRoles.map(role => (
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

                <div className="mt-4 flex flex-col gap-2 rounded-xl border border-stone-200 bg-stone-50 px-3 py-3 sm:flex-row sm:items-center sm:justify-between">
                  <div>
                    <div className="text-xs font-medium uppercase tracking-wide text-stone-400">
                      Unlock Action
                    </div>
                    <div className="mt-1 text-sm text-stone-800">{role.actionLabel}</div>
                  </div>
                  <div className="text-sm font-medium text-stone-600">{role.progressLabel}</div>
                </div>
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
};

export default Rewards;
