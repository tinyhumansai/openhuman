/**
 * Memory-embedding budget banner (#5324).
 *
 * Shell-mounted beside {@link GlobalUpsellBanner}, so the warning reaches the
 * user on whatever screen they are on rather than waiting for them to open
 * Memory Tree settings — the failure mode this issue is about.
 *
 * Escalation, matching the issue's acceptance criteria:
 *
 * | Consumption | Behaviour                                              |
 * | ----------- | ------------------------------------------------------ |
 * | ≥ 75%       | dismissible warning — "set up local embeddings or …"    |
 * | ≥ 90%       | non-dismissible warning with the same CTA              |
 * | exhausted   | non-dismissible, and memory has already stopped growing |
 *
 * Dismissal is per-session and per-level on purpose: dismissing the 75%
 * warning must not also silence the 90% escalation, or the user is back to a
 * silent failure. It is deliberately not persisted — a warning that survives
 * a restart it no longer applies to is worse than one shown twice.
 *
 * The CTA deep-links to the embeddings configuration screen. It never asks
 * the user to know what an embedding is: the copy names the two fixes (local
 * Ollama, own API key) and the button takes them to the one screen where both
 * are done.
 */
import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import {
  type EmbeddingBudgetLevel,
  useEmbeddingBudgetState,
} from '../../hooks/useEmbeddingBudgetState';
import { useT } from '../../lib/i18n/I18nContext';
import { showNativeNotification } from '../../lib/nativeNotifications/tauriBridge';
import UpsellBanner from './UpsellBanner';

/** Where both remediations (local Ollama, BYO key) are configured. */
export const EMBEDDINGS_SETTINGS_ROUTE = '/connections?tab=embeddings';

/** Only the early warning can be silenced; escalations cannot. */
function isDismissible(level: EmbeddingBudgetLevel): boolean {
  return level === 'warn';
}

/**
 * Module-scoped so the OS notification fires at most once per app session.
 * The banner re-renders on every usage poll; without this the user would get
 * a notification every 60s, which trains them to mute the app.
 */
let nativeNotificationSent = false;

/** Test seam — resets the once-per-session latch. */
export function __resetNativeNotificationLatchForTests() {
  nativeNotificationSent = false;
}

export default function MemoryEmbeddingBudgetBanner() {
  const { t } = useT();
  const navigate = useNavigate();
  const { level, pct } = useEmbeddingBudgetState();
  const [dismissedLevel, setDismissedLevel] = useState<EmbeddingBudgetLevel | null>(null);

  // Push an OS-level notification the first time the budget is actually spent.
  // The in-app banner and UserErrorCenter only reach a user who is looking at
  // the app; the whole point of this issue is that memory broke while nobody
  // was looking. Email is the backend's job (tracked separately) — this is the
  // client-side half.
  //
  // Fires only on `exhausted`, never on the 75%/90% warnings: those are not
  // yet a broken state, and an OS notification for them would be noise.
  useEffect(() => {
    if (level !== 'exhausted' || nativeNotificationSent) return;
    nativeNotificationSent = true;
    void showNativeNotification({
      title: t('memoryBudget.exhaustedTitle'),
      body: t('memoryBudget.exhaustedMessage'),
      tag: 'memory-embedding-budget-exhausted',
    });
  }, [level, t]);

  if (level === 'none') return null;
  if (dismissedLevel === level) return null;

  const isExhausted = level === 'exhausted';
  const title = isExhausted ? t('memoryBudget.exhaustedTitle') : t('memoryBudget.approachingTitle');
  const message = isExhausted
    ? t('memoryBudget.exhaustedMessage')
    : t('memoryBudget.approachingMessage').replace('{pct}', String(pct));

  return (
    <div className="relative z-20" data-testid="memory-embedding-budget-banner">
      <UpsellBanner
        variant="warning"
        title={title}
        message={message}
        ctaLabel={t('memoryBudget.cta')}
        rounded={false}
        dismissible={isDismissible(level)}
        onDismiss={() => setDismissedLevel(level)}
        onCtaClick={() => {
          navigate(EMBEDDINGS_SETTINGS_ROUTE);
        }}
      />
    </div>
  );
}
