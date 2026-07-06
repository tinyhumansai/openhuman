/**
 * TinyPlaceRoster — the instance roster: external agent sessions grouped by
 * harness (Claude / Codex / Gemini, then an "Other" catch-all), each a
 * selectable {@link InstanceCard}. Pinned master/subconscious windows are not
 * instances and are excluded here.
 *
 * Presentational only; the parent passes the session list, selection, and an
 * optional address→@handle map.
 */
import { useT } from '../../lib/i18n/I18nContext';
import type { HarnessType, SessionSummary } from '../../lib/orchestration/orchestrationClient';
import InstanceCard from './InstanceCard';

export interface TinyPlaceRosterProps {
  sessions: SessionSummary[];
  selectedId?: string;
  onSelect?: (sessionId: string) => void;
  /** Resolved address → `@handle` map (best-effort; address is the fallback). */
  handles?: Record<string, string | null>;
}

// Grouped in this order; brand names are identity, not translated UI copy.
const HARNESS_GROUPS: Array<{ key: HarnessType; label: string }> = [
  { key: 'claude', label: 'Claude' },
  { key: 'codex', label: 'Codex' },
  { key: 'gemini', label: 'Gemini' },
];

export default function TinyPlaceRoster({
  sessions,
  selectedId,
  onSelect,
  handles,
}: TinyPlaceRosterProps): React.ReactElement {
  const { t } = useT();

  // Instances are the non-pinned session windows.
  const instances = sessions.filter(s => !s.pinned && s.chatKind === 'session');

  const byHarness = (harness: HarnessType): SessionSummary[] =>
    instances.filter(s => s.harnessType === harness);
  const ungrouped = instances.filter(s => !s.harnessType);

  const groups = [
    ...HARNESS_GROUPS.map(g => ({ label: g.label, rows: byHarness(g.key) })),
    { label: t('tinyplaceOrchestration.roster.other'), rows: ungrouped },
  ].filter(g => g.rows.length > 0);

  return (
    <section data-testid="tinyplace-roster" className="min-w-0">
      <h4 className="px-3 pb-1 pt-3 text-[10px] font-semibold uppercase tracking-wide text-content-muted">
        {t('tinyplaceOrchestration.roster.instances')}
      </h4>
      {instances.length === 0 ? (
        <p
          data-testid="tinyplace-roster-empty"
          className="px-3 py-4 text-[11px] text-content-faint">
          {t('tinyplaceOrchestration.roster.empty')}
        </p>
      ) : (
        groups.map(group => (
          <div key={group.label}>
            <div className="px-3 pb-0.5 pt-2 text-[10px] font-semibold uppercase tracking-wide text-content-faint">
              {group.label}
            </div>
            {group.rows.map(session => (
              <InstanceCard
                key={session.sessionId}
                session={session}
                selected={session.sessionId === selectedId}
                handle={handles?.[session.agentId]}
                onSelect={onSelect ? () => onSelect(session.sessionId) : undefined}
              />
            ))}
          </div>
        ))
      )}
    </section>
  );
}
