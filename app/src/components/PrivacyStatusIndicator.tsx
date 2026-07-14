import { privacyModeLabelKey } from '../features/privacy/disclosureLabels';
import { useT } from '../lib/i18n/I18nContext';
import { useAppSelector } from '../store/hooks';

interface PrivacyStatusIndicatorProps {
  className?: string;
}

/**
 * Persistent privacy-status pill (#4437 / S3). Mirrors {@link ConnectionIndicator}
 * — an inline-flex chip with a coloured dot + tiny label. Shows the current
 * Privacy Mode plus whether the *active task* is staying on-device or sending
 * externally.
 *
 * The `external_transfer_pending` event only fires on an EXTERNAL transfer, so
 * "on-device" is inferred from the mode + the ABSENCE of a recent external
 * disclosure for the active thread — never from a positive "local" signal.
 * Renders nothing until the mode is hydrated so the pill is never misleading.
 */
const PrivacyStatusIndicator = ({ className = '' }: PrivacyStatusIndicatorProps) => {
  const { t } = useT();
  // Optional-chain the `privacy` slice — narrow test stores may omit it.
  const privacyMode = useAppSelector(state => state.privacy?.privacyMode ?? null);
  const selectedThreadId = useAppSelector(state => state.thread?.selectedThreadId ?? null);
  const disclosuresByThread = useAppSelector(state => state.privacy?.disclosuresByThread);

  if (!privacyMode) return null;

  // Local-only mode blocks external model calls (enforced core-side by S7), so
  // the active task is always on-device there regardless of any stale event.
  const hasExternalDisclosure = selectedThreadId
    ? (disclosuresByThread?.[selectedThreadId]?.some(d => d.isExternal) ?? false)
    : false;
  const isExternal = privacyMode !== 'local_only' && hasExternalDisclosure;

  const modeLabel = t(privacyModeLabelKey(privacyMode));
  const stateLabel = isExternal ? t('privacy.status.external') : t('privacy.status.local');
  const dotColor = isExternal ? 'bg-amber-500' : 'bg-sage-500';
  const textColor = isExternal ? 'text-amber-500' : 'text-sage-500';

  return (
    <div
      className={`inline-flex items-center gap-1.5 ${className}`}
      role="status"
      aria-label={t('privacy.status.ariaLabel')}
      title={`${modeLabel} · ${stateLabel}`}>
      <div className={`h-2 w-2 rounded-full ${dotColor} ${isExternal ? 'animate-pulse' : ''}`} />
      <span className={`text-[10px] font-medium ${textColor}`}>
        {modeLabel}
        <span className="text-content-faint"> · </span>
        {stateLabel}
      </span>
    </div>
  );
};

export default PrivacyStatusIndicator;
