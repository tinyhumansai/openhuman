/**
 * TeamActivityRail — the live teammate-message timeline for an agent team
 * (#3374 PR2).
 *
 * This is the surface's differentiator: a board of owned tasks is commodity,
 * but seeing the agents *talk to each other while they work* is the thing that
 * makes multi-agent coordination legible. Each entry is a `team_message` run
 * event resolved to `from → to` (member names, with the member's deterministic
 * colour) plus the message body. A `null` recipient means the message was sent
 * to the whole team.
 *
 * The rail renders inside a grid cell that collapses *under* the board on
 * narrow widths (handled by the parent tab's responsive grid), so it is always
 * visible on a wide Intelligence pane and stacks gracefully when cramped.
 */
import { useT } from '../../lib/i18n/I18nContext';
import type { AgentTeamMember, TeamMessage } from '../../services/api/agentTeamApi';
import { memberColor } from './memberColors';

interface TeamActivityRailProps {
  messages: TeamMessage[];
  members: AgentTeamMember[];
}

export function TeamActivityRail({ messages, members }: TeamActivityRailProps) {
  const { t } = useT();
  const memberById = new Map(members.map(m => [m.id, m]));

  const nameFor = (id: string | null): string => {
    if (!id) return t('intelligence.teams.activity.toTeam');
    return memberById.get(id)?.name ?? id;
  };

  return (
    <aside className="rounded-lg border border-stone-200 bg-white p-3 dark:border-neutral-800 dark:bg-neutral-900">
      <div className="mb-2 flex items-center justify-between">
        <h3 className="text-[11px] font-semibold uppercase tracking-wide text-stone-500 dark:text-neutral-400">
          {t('intelligence.teams.activity.title')}
        </h3>
        <span className="text-[10px] text-stone-400 dark:text-neutral-500">{messages.length}</span>
      </div>

      {messages.length === 0 ? (
        <p className="py-6 text-center text-[11px] text-stone-400 dark:text-neutral-500">
          {t('intelligence.teams.activity.empty')}
        </p>
      ) : (
        <div className="space-y-3">
          {messages.map(message => {
            const fromMember = memberById.get(message.payload.from);
            const fromName = fromMember?.name ?? message.payload.from;
            const color = memberColor(message.payload.from);
            return (
              <div key={`${message.runId}-${message.sequence}`} className="flex gap-2">
                <span
                  className="mt-0.5 inline-flex h-5 w-5 flex-none items-center justify-center rounded-full text-[9px] font-semibold text-white"
                  style={{ backgroundColor: color }}>
                  {fromName.charAt(0).toUpperCase()}
                </span>
                <div className="min-w-0">
                  <div className="text-[10px] text-stone-400 dark:text-neutral-500">
                    <b className="text-stone-600 dark:text-neutral-300">{fromName}</b>
                    {' → '}
                    {nameFor(message.payload.to)}
                  </div>
                  <p className="break-words text-[11px] leading-snug text-stone-700 dark:text-neutral-200">
                    {message.payload.content}
                  </p>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </aside>
  );
}
