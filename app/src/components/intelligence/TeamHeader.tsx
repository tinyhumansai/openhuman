/**
 * TeamHeader — the identity strip above a team's task board (#3374 PR2).
 *
 * Shows the team's summary (falling back to its lead agent id), the lead, a
 * task/member count line, and a compact roster of member chips. Each chip
 * carries the member's deterministic colour (so a member reads the same on the
 * header, the board border and the activity rail) and a status dot
 * (active / pending / idle / stopped). For the small teams this surface targets
 * (≈2–5 members) a header strip is the right density — a full sidebar would be
 * heavy furniture for three chips.
 */
import { useT } from '../../lib/i18n/I18nContext';
import type {
  AgentTeam,
  AgentTeamMember,
  AgentTeamMemberStatus,
} from '../../services/api/agentTeamApi';
import { memberColor } from './memberColors';

/** Status dot colour per member lifecycle state. */
const MEMBER_STATUS_DOT: Record<AgentTeamMemberStatus, string> = {
  active: 'bg-sage-500',
  pending: 'bg-amber-500',
  idle: 'bg-stone-400 dark:bg-neutral-500',
  stopped: 'bg-coral-500',
};

const MEMBER_STATUS_KEY: Record<AgentTeamMemberStatus, string> = {
  active: 'intelligence.teams.member.active',
  pending: 'intelligence.teams.member.pending',
  idle: 'intelligence.teams.member.idle',
  stopped: 'intelligence.teams.member.stopped',
};

interface TeamHeaderProps {
  team: AgentTeam;
  members: AgentTeamMember[];
  taskCount: number;
}

export function TeamHeader({ team, members, taskCount }: TeamHeaderProps) {
  const { t } = useT();
  const title = team.summary?.trim() || team.leadAgentId;

  return (
    <div className="rounded-lg border border-stone-200 bg-white px-3 py-2.5 dark:border-neutral-800 dark:bg-neutral-900">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="break-words text-sm font-semibold text-stone-800 dark:text-neutral-100">
            {title}
          </div>
          <div className="mt-0.5 text-[11px] text-stone-500 dark:text-neutral-400">
            {t('intelligence.teams.header.lead')}{' '}
            <span className="font-mono text-ocean-600 dark:text-ocean-300">{team.leadAgentId}</span>
            {' · '}
            {t('intelligence.teams.header.taskCount').replace('{count}', String(taskCount))}
            {' · '}
            {t('intelligence.teams.header.memberCount').replace('{count}', String(members.length))}
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-1">
          {members.map(member => (
            <span
              key={member.id}
              title={`${member.agentId ?? member.name} · ${t(MEMBER_STATUS_KEY[member.memberStatus])}`}
              className="inline-flex items-center gap-1 rounded-md border border-stone-200 bg-stone-50 px-1.5 py-0.5 text-[10px] text-stone-600 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-300">
              <span
                className="inline-flex h-3.5 w-3.5 flex-none items-center justify-center rounded-full text-[7px] font-semibold text-white"
                style={{ backgroundColor: memberColor(member.id) }}>
                {member.name.charAt(0).toUpperCase()}
              </span>
              <span className="max-w-[7rem] truncate">{member.name}</span>
              <span
                className={`h-1.5 w-1.5 flex-none rounded-full ${MEMBER_STATUS_DOT[member.memberStatus]}`}
              />
            </span>
          ))}
        </div>
      </div>
    </div>
  );
}
