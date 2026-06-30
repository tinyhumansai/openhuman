/**
 * TeamTaskBoard — the five-column kanban for a single agent team's tasks
 * (#3374 PR2).
 *
 * A sibling of the conversations `TaskKanbanBoard`, NOT a reuse of it: team
 * tasks are `AgentTeamTask` (owner / claimed-by / dependency graph / quality
 * gate) rather than the generic `TaskBoardCard`, so folding them into the
 * shared component would leak team concepts into the conversations board and
 * risk regressing it. This board deliberately speaks the same visual language —
 * the same five-column shell, the same amber/coral column accents, the same
 * house tokens — so it reads as the same product while owning the team-specific
 * affordances:
 *
 *   - owner colour left-border + owner chip (deterministic per-member colour)
 *   - "picked up by …" line when the claimer differs from the owner
 *   - dependency lock badge showing the count of unmet (non-`done`) deps
 *   - quality-gate badge (pending / passed / failed / unknown)
 *
 * Read-only: tasks are driven by agents over the `agent_team_*` RPCs. There is
 * no drag-and-drop or status mutation here (that is the agents' job).
 */
import { LuCircleCheck, LuLock, LuShieldCheck } from 'react-icons/lu';

import { useT } from '../../lib/i18n/I18nContext';
import type {
  AgentTeamMember,
  AgentTeamTask,
  AgentTeamTaskStatus,
} from '../../services/api/agentTeamApi';
import { memberColor, memberTint } from './memberColors';

/** The five columns, in display order. Mirrors the team task lifecycle. */
const COLUMN_DEFS: { status: AgentTeamTaskStatus; labelKey: string }[] = [
  { status: 'todo', labelKey: 'intelligence.teams.column.todo' },
  { status: 'ready', labelKey: 'intelligence.teams.column.ready' },
  { status: 'in_progress', labelKey: 'intelligence.teams.column.inProgress' },
  { status: 'blocked', labelKey: 'intelligence.teams.column.blocked' },
  { status: 'done', labelKey: 'intelligence.teams.column.done' },
];

/** Per-column accent — mirrors the conversations board's amber/coral tints. */
const COLUMN_ACCENT: Record<AgentTeamTaskStatus, string> = {
  todo: '',
  ready: '',
  in_progress: '',
  blocked:
    'border-l-2 border-l-coral-400 bg-coral-50/60 dark:bg-coral-500/5 dark:border-l-coral-500/60',
  done: '',
};

/** Known gate states → badge styling. Unknown strings fall back to neutral. */
const GATE_STYLE: Record<string, string> = {
  pending: 'bg-amber-50 text-amber-700 dark:bg-amber-500/10 dark:text-amber-300',
  passed: 'bg-sage-50 text-sage-700 dark:bg-sage-500/10 dark:text-sage-300',
  approved: 'bg-sage-50 text-sage-700 dark:bg-sage-500/10 dark:text-sage-300',
  failed: 'bg-coral-50 text-coral-700 dark:bg-coral-500/10 dark:text-coral-300',
  rejected: 'bg-coral-50 text-coral-700 dark:bg-coral-500/10 dark:text-coral-300',
};

const GATE_LABEL_KEY: Record<string, string> = {
  pending: 'intelligence.teams.gate.pending',
  passed: 'intelligence.teams.gate.passed',
  approved: 'intelligence.teams.gate.passed',
  failed: 'intelligence.teams.gate.failed',
  rejected: 'intelligence.teams.gate.failed',
};

/**
 * Count a task's unmet dependencies: dep ids that resolve to a known task whose
 * status is not yet `done`. Unknown ids are ignored (the backend rejects them
 * at assign time, so they shouldn't occur; guarding avoids a phantom lock).
 */
export function unmetDepCount(task: AgentTeamTask, byId: Map<string, AgentTeamTask>): number {
  let unmet = 0;
  for (const depId of task.dependsOn) {
    const dep = byId.get(depId);
    if (dep && dep.status !== 'done') unmet += 1;
  }
  return unmet;
}

interface TeamTaskBoardProps {
  tasks: AgentTeamTask[];
  members: AgentTeamMember[];
}

export function TeamTaskBoard({ tasks, members }: TeamTaskBoardProps) {
  const { t } = useT();

  const memberById = new Map(members.map(m => [m.id, m]));
  const taskById = new Map(tasks.map(task => [task.id, task]));

  const cardsByStatus = COLUMN_DEFS.reduce(
    (acc, column) => {
      acc[column.status] = [];
      return acc;
    },
    {} as Record<AgentTeamTaskStatus, AgentTeamTask[]>
  );
  for (const task of [...tasks].sort((a, b) => a.orderIndex - b.orderIndex)) {
    (cardsByStatus[task.status] ?? cardsByStatus.todo).push(task);
  }

  return (
    <div className="grid grid-cols-1 gap-2 sm:grid-cols-3 lg:grid-cols-5">
      {COLUMN_DEFS.map(column => {
        const cards = cardsByStatus[column.status];
        const accentClass = COLUMN_ACCENT[column.status];
        return (
          <section
            key={column.status}
            className={`min-w-0 rounded-lg bg-surface-muted p-2 ${accentClass}`}>
            <div className="mb-2 flex items-center justify-between gap-2">
              <h5 className="truncate text-[11px] font-medium text-content-secondary">
                {t(column.labelKey)}
              </h5>
              <span className="text-[10px] text-content-faint">{cards.length}</span>
            </div>
            <div className="space-y-2">
              {cards.length === 0 ? (
                <p className="py-2 text-center text-[10px] text-content-faint dark:text-neutral-600">
                  {t('intelligence.teams.emptyColumn')}
                </p>
              ) : (
                cards.map(task => (
                  <TeamTaskCard
                    key={task.id}
                    task={task}
                    memberById={memberById}
                    unmet={unmetDepCount(task, taskById)}
                  />
                ))
              )}
            </div>
          </section>
        );
      })}
    </div>
  );
}

function TeamTaskCard({
  task,
  memberById,
  unmet,
}: {
  task: AgentTeamTask;
  memberById: Map<string, AgentTeamMember>;
  unmet: number;
}) {
  const { t } = useT();
  const owner = task.ownerMemberId ? memberById.get(task.ownerMemberId) : undefined;
  const claimer = task.claimedByMemberId ? memberById.get(task.claimedByMemberId) : undefined;
  // Only surface the claimer line when it adds information — i.e. someone other
  // than the owner picked the task up.
  const showClaimer = claimer && claimer.id !== task.ownerMemberId;

  // The owner colour left-border is the at-a-glance "whose task is this" cue.
  const borderStyle = owner ? { borderLeft: `3px solid ${memberColor(owner.id)}` } : undefined;

  return (
    <article
      style={borderStyle}
      className="rounded-lg border border-line bg-surface px-2.5 py-2 shadow-sm">
      <p className="break-words text-xs font-medium leading-snug text-content">{task.title}</p>

      {showClaimer && (
        <p className="mt-0.5 text-[10px] text-content-muted">
          {t('intelligence.teams.pickedUpBy').replace('{name}', claimer.name)}
        </p>
      )}

      <div className="mt-1.5 flex flex-wrap items-center gap-1">
        {owner ? (
          <span
            style={{ backgroundColor: memberTint(owner.id), color: memberColor(owner.id) }}
            className="inline-flex max-w-full items-center gap-1 rounded px-1.5 py-0.5 text-[10px]">
            <span
              className="inline-flex h-3 w-3 flex-none items-center justify-center rounded-full text-[7px] font-semibold text-white"
              style={{ backgroundColor: memberColor(owner.id) }}>
              {owner.name.charAt(0).toUpperCase()}
            </span>
            <span className="truncate">{owner.name}</span>
          </span>
        ) : (
          <span className="rounded bg-surface-subtle px-1.5 py-0.5 text-[10px] italic text-content-faint">
            {t('intelligence.teams.unclaimed')}
          </span>
        )}

        {unmet > 0 && (
          <span
            title={t('intelligence.teams.depLockTitle').replace('{count}', String(unmet))}
            className="inline-flex items-center gap-1 rounded bg-surface-subtle px-1.5 py-0.5 text-[10px] text-content-secondary">
            <LuLock className="h-2.5 w-2.5" />
            {unmet}
          </span>
        )}

        <GateBadge gateStatus={task.gateStatus} gateReason={task.gateReason} />

        {task.evidence.length > 0 && (
          <span className="inline-flex items-center gap-1 rounded bg-sky-50 px-1.5 py-0.5 text-[10px] text-sky-700 dark:bg-sky-500/10 dark:text-sky-200">
            <LuCircleCheck className="h-2.5 w-2.5" />
            {task.evidence.length}
          </span>
        )}
      </div>

      {task.objective && (
        <p className="mt-1 break-words text-[11px] leading-snug text-content-muted">
          {task.objective}
        </p>
      )}
    </article>
  );
}

function GateBadge({ gateStatus, gateReason }: { gateStatus: string; gateReason?: string | null }) {
  const { t } = useT();
  // Defensive mapping: an unseen gate string must render as a neutral chip
  // showing its raw value, never crash or vanish.
  const style = GATE_STYLE[gateStatus] ?? 'bg-surface-subtle text-content-secondary';
  const label = GATE_LABEL_KEY[gateStatus]
    ? t(GATE_LABEL_KEY[gateStatus])
    : t('intelligence.teams.gate.label').replace('{status}', gateStatus);

  return (
    <span
      title={gateReason ?? undefined}
      className={`inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] ${style}`}>
      <LuShieldCheck className="h-2.5 w-2.5" />
      {label}
    </span>
  );
}
