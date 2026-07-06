/**
 * InstanceStatusDot — the core roster primitive: one colored dot that encodes an
 * agent instance's status at a glance, scannable across many rows.
 *
 * Five states (color + motion), matching the core's {@link InstanceStatus}:
 * running (ocean, pulsing) · idle (sage) · waiting-approval (amber) ·
 * errored (coral) · stopped (faint). The core derives only idle/stopped today;
 * the other three are wired ahead of the attention-queue / run-state work.
 *
 * Presentational only. Pass a translated `label` for the accessible name.
 */
import type { InstanceStatus } from '../../lib/orchestration/orchestrationClient';

export interface InstanceStatusDotProps {
  status: InstanceStatus;
  /** Accessible label (already translated by the caller). */
  label?: string;
}

const TONE: Record<InstanceStatus, string> = {
  running: 'bg-ocean-500 animate-pulse',
  idle: 'bg-sage-500',
  'waiting-approval': 'bg-amber-500',
  errored: 'bg-coral-500',
  stopped: 'bg-content-faint',
};

export default function InstanceStatusDot({
  status,
  label,
}: InstanceStatusDotProps): React.ReactElement {
  return (
    <span
      role="img"
      aria-label={label ?? status}
      title={label ?? status}
      data-testid="instance-status-dot"
      data-status={status}
      className={`inline-block h-2 w-2 flex-none rounded-full ${TONE[status]}`}
    />
  );
}
