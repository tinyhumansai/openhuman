/**
 * A one-time banner on Memory → Provider announcing the move from TinyCortex
 * to CortexDB. Dismissing it is remembered per user (`userScopedStorage`), so
 * it never comes back for that account.
 *
 * debug logging: DEBUG=openhuman:memory:announcement
 */
import debug from 'debug';
import { X } from 'lucide-react';
import { useEffect, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { userScopedStorage } from '../../store/userScopedStorage';
import { Alert, AlertDescription, AlertTitle, Button } from '../ui';

const log = debug('openhuman:memory:announcement');

/** Storage key for the dismissal; bump the suffix to show a new announcement. */
export const CORTEX_ANNOUNCEMENT_KEY = 'memory.cortexdbAnnouncement.dismissed.v1';

export default function MemoryCortexAnnouncement() {
  const { t } = useT();
  // Unknown until storage answers, so a dismissed banner never flashes.
  const [dismissed, setDismissed] = useState<boolean | null>(null);

  useEffect(() => {
    let cancelled = false;
    userScopedStorage
      .getItem(CORTEX_ANNOUNCEMENT_KEY)
      .then(value => {
        if (!cancelled) setDismissed(value === 'true');
      })
      .catch(err => {
        log('read failed: %o', err);
        if (!cancelled) setDismissed(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  if (dismissed !== false) return null;

  const dismiss = () => {
    setDismissed(true);
    log('dismissed');
    void userScopedStorage.setItem(CORTEX_ANNOUNCEMENT_KEY, 'true');
  };

  return (
    <Alert
      variant="info"
      aria-labelledby="memory-cortex-announcement-title"
      data-testid="memory-cortex-announcement"
      className="items-center pr-11">
      <span aria-hidden className="shrink-0 text-2xl leading-none">
        🎉
      </span>
      <div className="min-w-0 flex-1 space-y-0.5">
        <AlertTitle id="memory-cortex-announcement-title">
          {t('memoryPage.announcement.title')}
        </AlertTitle>
        <AlertDescription className="text-xs">
          {t('memoryPage.announcement.retired')}{' '}
          <strong
            className="rounded bg-primary-500/15 px-1 py-px font-semibold text-content"
            data-testid="memory-cortex-announcement-highlight">
            {t('memoryPage.announcement.highlight')}
          </strong>{' '}
          {t('memoryPage.announcement.rest')}
        </AlertDescription>
      </div>
      <Button
        size="xs"
        variant="tertiary"
        iconOnly
        analyticsId="memory-cortex-announcement-dismiss"
        data-testid="memory-cortex-announcement-dismiss"
        aria-label={t('memoryPage.announcement.dismiss')}
        className="absolute top-2.5 right-2.5 opacity-70 hover:opacity-100"
        onClick={dismiss}>
        <X className="h-3.5 w-3.5" aria-hidden />
      </Button>
    </Alert>
  );
}
