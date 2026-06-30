import type { ReactNode } from 'react';

import type { ToolTimelineEntryStatus } from '../../../store/chatRuntimeSlice';

/**
 * Small "spark" glyph used as each agent's node on the timeline rail —
 * mirrors the Figma "Intelligence" icon. Inherits `currentColor` so the
 * caller controls its tone (muted while running, solid when done).
 */
export function AgentSparkIcon({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 12 12"
      width="12"
      height="12"
      aria-hidden
      className={className}
      focusable="false">
      <path
        d="M6 0.4 L7.25 4.75 L11.6 6 L7.25 7.25 L6 11.6 L4.75 7.25 L0.4 6 L4.75 4.75 Z"
        fill="currentColor"
      />
    </svg>
  );
}

/**
 * Map a timeline row's lifecycle status to the agent-name text treatment.
 *
 * The Figma "Agentic task insights" design conveys per-agent progress
 * through the *name text* rather than a progress bar: an in-flight agent
 * pulses in a muted tone, a finished agent reads solid/full-strength, and
 * a failed agent is tinted with the coral error token. (Per product
 * direction — no numeric progress signal exists from the core, so we never
 * fabricate one.)
 */
export function agentNameTone(status: ToolTimelineEntryStatus | undefined): string {
  switch (status) {
    case 'success':
      // Done — full-strength foreground ("full white" in the dark mockup).
      return 'text-content-secondary dark:text-content';
    case 'error':
      return 'text-coral-600 dark:text-coral-300';
    case 'awaiting_user':
      return 'animate-pulse text-amber-600 dark:text-amber-300';
    case 'cancelled':
      // Cancelled — terminal, so muted but NOT pulsing (it isn't in progress).
      return 'text-content-faint';
    default:
      // running / unknown — in progress: muted + blinking.
      return 'animate-pulse text-content-faint';
  }
}

/**
 * One row on the agent-insights timeline rail: a left column carrying the
 * spark node icon plus the vertical connector that threads consecutive
 * agents together, and an indented content column for the row body.
 *
 * The connector is drawn as two absolutely-positioned segments (above /
 * below the icon) so the line visually breaks at each node and is clipped
 * at the first/last rows — producing the continuous-but-segmented rail in
 * the Figma frames. The icon sits on an opaque chip matching the chat
 * surface so the line reads as passing *behind* it.
 */
export function AgentTimelineRail({
  isFirst = false,
  isLast = false,
  icon,
  iconClassName,
  children,
}: {
  isFirst?: boolean;
  isLast?: boolean;
  /** Override the default spark glyph (e.g. the "thoughts" reasoning row). */
  icon?: ReactNode;
  /** Tone applied to the default spark glyph. */
  iconClassName?: string;
  children: ReactNode;
}) {
  return (
    <div className="relative flex gap-2.5" data-testid="agent-timeline-row">
      {/* Left rail: connector segments + spark node */}
      <div className="relative flex w-3 shrink-0 justify-center">
        {!isFirst ? (
          <span
            aria-hidden
            className="absolute top-0 left-1/2 h-[9px] w-px -translate-x-1/2 bg-surface-strong"
          />
        ) : null}
        {!isLast ? (
          <span
            aria-hidden
            className="absolute top-[9px] bottom-0 left-1/2 w-px -translate-x-1/2 bg-surface-strong"
          />
        ) : null}
        <span className="relative z-10 mt-0.5 flex h-3 w-3 items-center justify-center bg-[#f6f6f6] dark:bg-surface-canvas">
          {icon ?? <AgentSparkIcon className={iconClassName ?? 'text-content-faint'} />}
        </span>
      </div>
      <div className="min-w-0 flex-1 pb-2">{children}</div>
    </div>
  );
}
