/**
 * RelayBadge — a small always-visible pill naming the tiny.place relay the core
 * is talking to (STAGING / PRODUCTION).
 *
 * Why it exists: during live debugging, claudebot sat on `staging-api.tiny.place`
 * while the core silently defaulted to prod `api.tiny.place`, producing a mystery
 * 404. Nothing on screen said which relay either side used. Surfacing the target
 * relay as a first-class chip makes a mismatch obvious instead of opaque.
 *
 * Presentational only — the parent fetches {@link RelayInfo} via
 * `orchestrationClient.relayInfo()` and passes it down.
 */
import { useT } from '../../lib/i18n/I18nContext';
import type { RelayInfo } from '../../lib/orchestration/orchestrationClient';

export interface RelayBadgeProps {
  relay: RelayInfo | null;
}

export default function RelayBadge({ relay }: RelayBadgeProps): React.ReactElement | null {
  const { t } = useT();
  if (!relay) return null;

  const isStaging = relay.network === 'staging';
  const label = isStaging
    ? t('tinyplaceOrchestration.relay.staging')
    : t('tinyplaceOrchestration.relay.prod');

  // Staging is the everyday dev target (amber, "you're on the test relay");
  // production is the live one (sage). Colour, not just text, so it's scannable.
  const tone = isStaging
    ? 'border-amber-400 text-amber-700 dark:border-amber-500/60 dark:text-amber-300'
    : 'border-sage-400 text-sage-700 dark:border-sage-500/60 dark:text-sage-300';

  return (
    <span
      data-testid="tinyplace-relay-badge"
      data-network={relay.network}
      title={relay.baseUrl}
      className={`inline-flex flex-none items-center rounded-md border px-1.5 py-0.5 font-mono text-[10px] font-bold uppercase tracking-wide ${tone}`}>
      {label}
    </span>
  );
}
