import debug from 'debug';
import { type FC, useMemo } from 'react';

import type { ToolTimelineEntry, ToolTimelineEntryStatus } from '../../store/chatRuntimeSlice';
import { formatToolName } from '../../utils/toolTimelineFormatting';
import { type MascotFace, RiveMascot } from './Mascot';
import type { MascotColor } from './Mascot/mascotPalette';

const subMascotLog = debug('human:sub-mascots');

const MAX_SUB_MASCOTS = 5;
const ACTIVITY_LIMIT = 74;

const SUB_MASCOT_COLORS: readonly MascotColor[] = ['yellow', 'navy', 'burgundy', 'black'] as const;

const POSITIONS = [
  { left: '72%', top: '18%' },
  { left: '24%', top: '20%' },
  { left: '80%', top: '62%' },
  { left: '18%', top: '64%' },
  { left: '50%', top: '10%' },
] as const;

export interface SubMascotModel {
  id: string;
  agentId: string;
  label: string;
  status: ToolTimelineEntryStatus;
  face: MascotFace;
  activity: string;
  color: MascotColor;
  position: (typeof POSITIONS)[number];
}

export interface SubMascotLayerProps {
  entries: ToolTimelineEntry[];
}

function hashString(value: string): number {
  let hash = 0;
  for (let i = 0; i < value.length; i += 1) {
    hash = (hash * 31 + value.charCodeAt(i)) >>> 0;
  }
  return hash;
}

function truncateActivity(value: string): string {
  const trimmed = value.trim().replace(/\s+/g, ' ');
  if (trimmed.length <= ACTIVITY_LIMIT) return trimmed;
  return `${trimmed.slice(0, ACTIVITY_LIMIT - 3).trimEnd()}...`;
}

function humanizeAgentId(value: string): string {
  const cleaned = value
    .replace(/^subagent:/, '')
    .replace(/[_-]+/g, ' ')
    .trim();
  if (!cleaned) return 'Subagent';
  return cleaned.replace(/\b\w/g, ch => ch.toUpperCase());
}

function faceForStatus(status: ToolTimelineEntryStatus): MascotFace {
  switch (status) {
    case 'success':
      return 'happy';
    case 'error':
      return 'concerned';
    case 'running':
    default:
      return 'thinking';
  }
}

function activityForEntry(entry: ToolTimelineEntry): string {
  const subagent = entry.subagent;
  if (!subagent) return 'Starting';

  if (entry.status === 'success') {
    return subagent.outputChars ? `Completed ${subagent.outputChars} chars` : 'Completed';
  }

  if (entry.status === 'error') {
    return 'Needs attention';
  }

  const lastRunningTool = [...subagent.toolCalls].reverse().find(call => call.status === 'running');
  if (lastRunningTool) {
    return `Using ${formatToolName(lastRunningTool.toolName)}`;
  }

  if (subagent.childIteration) {
    return subagent.childMaxIterations
      ? `Iteration ${subagent.childIteration}/${subagent.childMaxIterations}`
      : `Step ${subagent.childIteration}`;
  }

  if (entry.detail?.trim()) {
    return truncateActivity(entry.detail);
  }

  return 'Starting';
}

export function subMascotModelsFromTimeline(entries: ToolTimelineEntry[]): SubMascotModel[] {
  return entries
    .filter(
      entry =>
        entry.subagent &&
        entry.name.startsWith('subagent:') &&
        // Once a subagent's task is done (success or error), drop it from the
        // strip rather than letting completed mascots linger and crowd the
        // bottom. Only actively-running subagents are surfaced.
        entry.status === 'running'
    )
    .slice(-MAX_SUB_MASCOTS)
    .map((entry, index) => {
      const subagent = entry.subagent!;
      const agentId = subagent.agentId || entry.name.replace(/^subagent:/, '') || 'subagent';
      const colorIndex = hashString(`${subagent.taskId}:${agentId}`) % SUB_MASCOT_COLORS.length;
      return {
        id: entry.id,
        agentId,
        label: entry.subagent?.displayName ?? entry.displayName ?? humanizeAgentId(agentId),
        status: entry.status,
        face: faceForStatus(entry.status),
        activity: activityForEntry(entry),
        color: SUB_MASCOT_COLORS[colorIndex],
        position: POSITIONS[index % POSITIONS.length],
      };
    });
}

export const SubMascotLayer: FC<SubMascotLayerProps> = ({ entries }) => {
  const models = useMemo(() => subMascotModelsFromTimeline(entries), [entries]);

  if (models.length === 0) return null;

  subMascotLog(
    'render count=%d states=%o',
    models.length,
    models.map(model => `${model.agentId}:${model.status}`)
  );

  return (
    <div
      className="pointer-events-none absolute inset-x-0 bottom-0 z-10 flex justify-center"
      data-testid="sub-mascot-layer"
      aria-live="polite">
      <div className="flex items-end justify-center gap-3 px-3 pb-2 max-w-full overflow-x-auto">
        {models.map(model => (
          <div
            key={model.id}
            role="status"
            aria-label={`${model.label} subagent ${model.status}`}
            data-testid="sub-mascot"
            data-status={model.status}
            className="flex flex-col items-center w-[72px] flex-shrink-0">
            <div
              className={[
                'relative w-[56px] h-[56px] transition-opacity duration-500',
                model.status === 'running' ? 'opacity-100' : 'opacity-75',
              ].join(' ')}>
              <div className="drop-shadow-[0_6px_12px_rgba(15,23,42,0.18)]">
                <RiveMascot size="100%" face={model.face} />
              </div>
            </div>
            <div
              className="mt-1 max-w-[88px] rounded-md border border-white/70 bg-surface/85 px-1.5 py-0.5 text-center text-[9px] leading-tight text-content-secondary shadow-soft backdrop-blur dark:border-line-strong dark:bg-surface/85 dark:text-neutral-200"
              data-testid="sub-mascot-bubble"
              title={`${model.label} — ${model.activity}`}>
              <div className="truncate font-medium">{model.label}</div>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
};
