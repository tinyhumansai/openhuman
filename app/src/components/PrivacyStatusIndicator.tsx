import debug from 'debug';
import { useEffect } from 'react';

import { privacyModeLabelKey } from '../features/privacy/disclosureLabels';
import { useT } from '../lib/i18n/I18nContext';
import { useAppSelector } from '../store/hooks';

const pillLog = debug('privacy:pill');

interface PrivacyStatusIndicatorProps {
  className?: string;
}

/**
 * Persistent privacy-status pill (#4437 / S3). Mirrors {@link ConnectionIndicator}
 * — an inline-flex chip with a coloured dot + tiny label. Shows the current
 * Privacy Mode plus whether the *active task* is staying on-device or sending
 * externally.
 *
 * The off-device sub-state is driven by `privacy.activeExternalByThread` — a
 * flag set when an `external_transfer_pending` disclosure arrives and cleared
 * on the turn boundary by ChatRuntimeProvider. It is deliberately NOT derived
 * from the dismissible disclosure ledger: reading the ledger let a "Got it"
 * dismissal flip the pill on-device mid-transfer, and let a stale historical
 * entry pin it off-device during later local turns. "On-device" is therefore
 * the ABSENCE of a live external transfer, never a positive "local" signal.
 * Renders nothing (a self-nulling leading separator + chip) until the mode is
 * hydrated so the pill is never misleading.
 */
const PrivacyStatusIndicator = ({ className = '' }: PrivacyStatusIndicatorProps) => {
  const { t } = useT();
  // Optional-chain the `privacy` slice — narrow test stores may omit it.
  const privacyMode = useAppSelector(state => state.privacy?.privacyMode ?? null);
  const selectedThreadId = useAppSelector(state => state.thread?.selectedThreadId ?? null);
  const activeExternalByThread = useAppSelector(state => state.privacy?.activeExternalByThread);

  // Local-only mode blocks external model calls (enforced core-side by S7), so
  // the active task is always on-device there regardless of any stale flag.
  const hasActiveExternalTransfer = selectedThreadId
    ? (activeExternalByThread?.[selectedThreadId] ?? false)
    : false;
  const localOnlyOverride = privacyMode === 'local_only';
  const isExternal = !localOnlyOverride && hasActiveExternalTransfer;

  // Diagnostics for the privacy-state flow (grep `privacy:pill`). Log only the
  // derived booleans/status transitions — never provider payloads, disclosure
  // contents, or user PII. Fires on transitions of the resolved state.
  useEffect(() => {
    pillLog(
      '[privacy:pill] derive hydrated=%s mode=%s hasThread=%s localOnlyOverride=%s activeExternal=%s isExternal=%s',
      String(privacyMode != null),
      privacyMode ?? 'none',
      String(selectedThreadId != null),
      String(localOnlyOverride),
      String(hasActiveExternalTransfer),
      String(isExternal)
    );
  }, [privacyMode, selectedThreadId, localOnlyOverride, hasActiveExternalTransfer, isExternal]);

  if (!privacyMode) return null;

  const modeLabel = t(privacyModeLabelKey(privacyMode));
  const stateLabel = isExternal ? t('privacy.status.external') : t('privacy.status.local');
  const dotColor = isExternal ? 'bg-amber-500' : 'bg-sage-500';
  const textColor = isExternal ? 'text-amber-500' : 'text-sage-500';

  // The leading separator travels WITH the pill so the sidebar footer never
  // renders a dangling `· ·` while the pill is un-hydrated (returns null). See
  // AppSidebar — the version item owns the separator that follows the pill.
  // Separator + chip are grouped in one inline-flex item so the footer's
  // flex-wrap can never split the leading `·` onto its own line — they wrap
  // together or not at all.
  return (
    <span className="inline-flex items-center gap-2">
      <span aria-hidden="true" className="text-[10px] text-content-faint">
        &middot;
      </span>
      <div
        className={`inline-flex items-center gap-1.5 ${className}`}
        role="status"
        aria-label={`${t('privacy.status.ariaLabel')}: ${modeLabel} · ${stateLabel}`}
        title={`${modeLabel} · ${stateLabel}`}>
        <div className={`h-2 w-2 rounded-full ${dotColor} ${isExternal ? 'animate-pulse' : ''}`} />
        <span className={`text-[10px] font-medium ${textColor}`}>
          {modeLabel}
          <span className="text-content-faint"> · </span>
          {stateLabel}
        </span>
      </div>
    </span>
  );
};

export default PrivacyStatusIndicator;
