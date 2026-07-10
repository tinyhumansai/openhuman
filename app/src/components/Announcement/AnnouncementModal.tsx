import { useT } from '../../lib/i18n/I18nContext';
import type { Announcement, AnnouncementSeverity } from '../../services/announcementService';

interface AnnouncementModalProps {
  announcement: Announcement;
  onDismiss: () => void;
}

// Severity drives the accent band + icon tint. Tokens mirror the app palette
// (primary/amber/coral) used elsewhere (see InitProgressScreen).
const severityAccent: Record<AnnouncementSeverity, string> = {
  INFO: 'border-primary-500/30 bg-primary-500/10 text-primary-300',
  WARNING: 'border-amber-500/30 bg-amber-500/10 text-amber-300',
  CRITICAL: 'border-coral-500/30 bg-coral-500/10 text-coral-300',
};

const severityLabel: Record<AnnouncementSeverity, string> = {
  INFO: 'Info',
  WARNING: 'Important',
  CRITICAL: 'Critical',
};

/**
 * One-shot announcement banner shown over the app after harness init. Title,
 * body, and the optional CTA are backend-provided (not i18n); only the dismiss
 * affordance is localized.
 */
export default function AnnouncementModal({ announcement, onDismiss }: AnnouncementModalProps) {
  const { t } = useT();
  const accent = severityAccent[announcement.severity];

  const openCta = () => {
    if (!announcement.cta) {
      return;
    }
    // Open externally; never navigate the app shell to a backend-provided URL.
    window.open(announcement.cta.url, '_blank', 'noopener,noreferrer');
    onDismiss();
  };

  return (
    <div
      className="fixed inset-0 z-[9998] flex items-center justify-center bg-stone-950/90 p-4 backdrop-blur-sm"
      data-testid="announcement-overlay">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="announcement-title"
        className="w-full max-w-md rounded-2xl border border-stone-700/60 bg-stone-900 p-6 shadow-2xl">
        <span
          className={`inline-flex items-center rounded-full border px-2.5 py-0.5 text-[11px] font-medium ${accent}`}>
          {severityLabel[announcement.severity]}
        </span>

        <h2 id="announcement-title" className="mt-3 text-lg font-semibold text-white">
          {announcement.title}
        </h2>
        <p className="mt-2 whitespace-pre-wrap text-sm text-stone-300">{announcement.body}</p>

        <div className="mt-6 flex justify-end gap-2">
          <button
            type="button"
            onClick={onDismiss}
            data-testid="announcement-dismiss"
            className="rounded-lg border border-stone-700 px-3 py-1.5 text-sm text-stone-300 hover:bg-stone-800 hover:text-white">
            {t('announcement.gotIt')}
          </button>
          {announcement.cta && (
            <button
              type="button"
              onClick={openCta}
              data-testid="announcement-cta"
              className="rounded-lg bg-primary-500 px-3 py-1.5 text-sm font-medium text-white hover:bg-primary-500/90">
              {announcement.cta.label}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
