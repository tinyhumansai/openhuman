import debug from 'debug';
import { type FC, useMemo } from 'react';

import type { ToolTimelineEntry, ToolTimelineEntryStatus } from '../../store/chatRuntimeSlice';
import { Ghosty, type MascotFace } from './Mascot';

const subMascotLog = debug('human:sub-mascots');

const MAX_SUB_MASCOTS = 5;
const ACTIVITY_LIMIT = 74;

const SUB_MASCOT_COLORS = [
  '#4A83DD',
  '#5C9B75',
  '#D9854B',
  '#B8657A',
  '#6E7BBD',
  '#4A9A9A',
] as const;

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
  color: string;
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

function humanizeIdentifier(value: string): string {
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
    return `Using ${humanizeIdentifier(lastRunningTool.toolName)}`;
  }

  if (subagent.childIteration) {
    return subagent.childMaxIterations
      ? `Iteration ${subagent.childIteration}/${subagent.childMaxIterations}`
      : `Iteration ${subagent.childIteration}`;
  }

  if (entry.detail?.trim()) {
    return truncateActivity(entry.detail);
  }

  return 'Starting';
}

export function subMascotModelsFromTimeline(entries: ToolTimelineEntry[]): SubMascotModel[] {
  return entries
    .filter(entry => entry.subagent && entry.name.startsWith('subagent:'))
    .slice(-MAX_SUB_MASCOTS)
    .map((entry, index) => {
      const subagent = entry.subagent!;
      const agentId = subagent.agentId || entry.name.replace(/^subagent:/, '') || 'subagent';
      const colorIndex = hashString(`${subagent.taskId}:${agentId}`) % SUB_MASCOT_COLORS.length;
      return {
        id: entry.id,
        agentId,
        label: humanizeIdentifier(agentId),
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
      className="pointer-events-none absolute inset-0 z-10"
      data-testid="sub-mascot-layer"
      aria-live="polite">
      {models.map(model => (
        <div
          key={model.id}
          role="status"
          aria-label={`${model.label} subagent ${model.status}`}
          data-testid="sub-mascot"
          data-status={model.status}
          className="absolute w-[clamp(78px,18%,128px)]"
          style={{
            left: model.position.left,
            top: model.position.top,
            transform: 'translate(-50%, -50%)',
          }}>
          <div
            className={[
              'relative transition-opacity duration-500',
              model.status === 'running' ? 'opacity-100' : 'opacity-85',
            ].join(' ')}>
            <div className="drop-shadow-[0_10px_20px_rgba(15,23,42,0.22)]">
              <Ghosty
                size="100%"
                idPrefix={`sub-mascot-${model.id.replace(/[^a-zA-Z0-9_-]/g, '-')}`}
                bodyColor={model.color}
                face={model.face}
              />
            </div>
            <div
              className="absolute left-1/2 top-[78%] w-[min(168px,42vw)] -translate-x-1/2 rounded-lg border border-white/70 bg-white/90 px-2 py-1 text-center text-[11px] leading-tight text-stone-700 shadow-soft backdrop-blur dark:border-neutral-700 dark:bg-neutral-900/90 dark:text-neutral-100"
              data-testid="sub-mascot-bubble">
              <div className="truncate font-medium">{model.label}</div>
              <div
                className="mt-0.5 overflow-hidden text-[10px] text-stone-500 dark:text-neutral-300"
                style={{ display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical' }}>
                {model.activity}
              </div>
            </div>
          </div>
        </div>
      ))}
    </div>
  );
};
