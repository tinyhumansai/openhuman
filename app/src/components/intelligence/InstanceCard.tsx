/**
 * InstanceCard — one roster row for an agent instance: harness glyph + status
 * dot + identity + one-line current task + unread pill.
 *
 * Presentational only; the parent supplies the {@link SessionSummary} and an
 * optional resolved `@handle` (the raw address is the fallback).
 */
import { useT } from '../../lib/i18n/I18nContext';
import type { InstanceStatus, SessionSummary } from '../../lib/orchestration/orchestrationClient';
import HarnessGlyph, { type GlyphKind } from './HarnessGlyph';
import InstanceStatusDot from './InstanceStatusDot';

export interface InstanceCardProps {
  session: SessionSummary;
  selected?: boolean;
  onSelect?: () => void;
  /** Resolved `@handle` for the peer, if known (address is the fallback). */
  handle?: string | null;
}

const STATUS_LABEL_KEY: Record<InstanceStatus, string> = {
  running: 'tinyplaceOrchestration.status.running',
  idle: 'tinyplaceOrchestration.status.idle',
  'waiting-approval': 'tinyplaceOrchestration.status.waitingApproval',
  errored: 'tinyplaceOrchestration.status.errored',
  stopped: 'tinyplaceOrchestration.status.stopped',
};

function shortAddress(address: string): string {
  if (address.length <= 14) return address;
  return `${address.slice(0, 6)}…${address.slice(-5)}`;
}

export default function InstanceCard({
  session,
  selected,
  onSelect,
  handle,
}: InstanceCardProps): React.ReactElement {
  const { t } = useT();
  const glyph: GlyphKind = session.harnessType ?? 'openhuman';
  const identity = handle ? `@${handle}` : (session.label ?? shortAddress(session.agentId));

  return (
    <button
      type="button"
      data-testid={`instance-card-${session.sessionId}`}
      data-selected={selected ? 'true' : 'false'}
      onClick={onSelect}
      className={`flex w-full items-center gap-3 border-l-2 px-3 py-2 text-left transition hover:bg-surface-hover ${
        selected ? 'border-ocean-500 bg-surface-muted' : 'border-transparent'
      }`}>
      <HarnessGlyph harness={glyph} />
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-1.5">
          <InstanceStatusDot status={session.status} label={t(STATUS_LABEL_KEY[session.status])} />
          <span className="truncate text-xs font-semibold text-content">{identity}</span>
        </span>
        {session.currentTask ? (
          <span className="mt-0.5 block truncate text-[11px] text-content-muted">
            {session.currentTask}
          </span>
        ) : null}
      </span>
      {session.unread > 0 ? (
        <span
          data-testid="instance-card-unread"
          className="flex-none rounded-full bg-ocean-500 px-1.5 py-0.5 text-[10px] font-semibold text-content-inverted">
          {session.unread}
        </span>
      ) : null}
    </button>
  );
}
