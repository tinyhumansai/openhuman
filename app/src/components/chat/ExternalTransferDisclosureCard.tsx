import type React from 'react';

import { dataKindLabelKey, reasonLabelKey } from '../../features/privacy/disclosureLabels';
import { useT } from '../../lib/i18n/I18nContext';
import { useAppDispatch } from '../../store/hooks';
import { dismissDisclosureForThread, type PrivacyDisclosure } from '../../store/privacySlice';
import Button from '../ui/Button';

interface Props {
  threadId: string;
  disclosure: PrivacyDisclosure;
}

/**
 * In-chat disclosure card for a pending external transfer (#4437 / S3).
 *
 * DISCLOSURE ONLY — it tells the user, at the moment of use, exactly what data
 * is leaving the device, where to, and why (epic #4256 AC1). There is NO
 * approve/deny arm; the only action is dismissal (that gate is S4 #4438). The
 * card mirrors {@link ApprovalRequestCard} / `PlanReviewCard`: rendered above
 * the composer for the active thread, off the privacy slice.
 */
export const ExternalTransferDisclosureCard: React.FC<Props> = ({ threadId, disclosure }) => {
  const { t } = useT();
  const dispatch = useAppDispatch();

  // Friendly, comma-joined data-kind labels (never the raw enum). An empty
  // list (metadata-only transfer) falls back to a generic "data" label so the
  // sentence never reads "… send  to …".
  const kinds =
    disclosure.dataKinds.length > 0
      ? disclosure.dataKinds
          .map(kind => t(dataKindLabelKey(kind)))
          .join(t('privacy.disclosure.kindSeparator'))
      : t('privacy.disclosure.kind.unknown');

  // Destination = human service name + the public provider slug for precision.
  const destination = `${disclosure.service} (${disclosure.providerSlug})`;
  const reason = t(reasonLabelKey(disclosure.reason));

  // Single translatable sentence with {placeholders} — the I18n layer has no
  // interpolation, so fill them here (keeps word order translatable per-locale).
  const body = t('privacy.disclosure.body')
    .replace('{kinds}', kinds)
    .replace('{destination}', destination)
    .replace('{reason}', reason);

  const onDismiss = () => {
    dispatch(dismissDisclosureForThread({ threadId, id: disclosure.id }));
  };

  return (
    <div
      role="status"
      aria-label={t('privacy.disclosure.ariaLabel')}
      className="rounded-xl border border-primary-200 bg-primary-50 p-3 text-sm shadow-sm dark:border-primary-800 dark:bg-primary-950">
      <div className="flex items-start gap-2">
        <span aria-hidden className="text-base leading-none text-primary-700 dark:text-primary-200">
          🛜
        </span>
        <div className="min-w-0 flex-1">
          <p className="font-semibold text-primary-900 dark:text-primary-100">
            {t('privacy.disclosure.title')}
          </p>
          <p className="mt-1 break-words text-primary-800/90 dark:text-primary-200/90">{body}</p>

          <div className="mt-3 flex flex-wrap items-center gap-2">
            <Button
              variant="secondary"
              size="sm"
              data-analytics-id="privacy-disclosure-dismiss"
              onClick={onDismiss}>
              {t('privacy.disclosure.dismiss')}
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
};

export default ExternalTransferDisclosureCard;
