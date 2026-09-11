/**
 * OS-level notification for an exhausted memory-embedding budget (#5324).
 *
 * Separate from the in-app notice because it serves the opposite case: the
 * panel and the FAB only reach a user who is *looking* at the app, and the
 * failure this issue is about is memory silently stopping while nobody was.
 * Email is the backend's half; this is the client's.
 *
 * Fires only on `exhausted`, never on the 75%/90% warnings — those are not yet
 * a broken state, and an OS notification for them would be noise.
 */
import { useEffect } from 'react';

import { useEmbeddingBudgetState } from '../../hooks/useEmbeddingBudgetState';
import { useT } from '../../lib/i18n/I18nContext';
import { showNativeNotification } from '../../lib/nativeNotifications/tauriBridge';

/**
 * Module-scoped so the notification fires at most once per app session. The
 * budget state re-reads on every usage poll; without this the user would get a
 * notification every 60s, which trains them to mute the app.
 */
let nativeNotificationSent = false;

/** Test seam — resets the once-per-session latch. */
export function __resetNativeNotificationLatchForTests() {
  nativeNotificationSent = false;
}

export function useEmbeddingBudgetNativeNotice(): void {
  const { t } = useT();
  const { level } = useEmbeddingBudgetState();

  useEffect(() => {
    if (level !== 'exhausted' || nativeNotificationSent) return;
    nativeNotificationSent = true;
    void showNativeNotification({
      title: t('memoryBudget.exhaustedTitle'),
      body: t('memoryBudget.exhaustedMessage'),
      tag: 'memory-embedding-budget-exhausted',
    });
  }, [level, t]);
}
