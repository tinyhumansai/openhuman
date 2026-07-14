import { useT } from '../../../lib/i18n/I18nContext';

/**
 * The "this is a preview, not live data" banner shown across the Orchestration
 * scale showcase (graph / tasks / network) for users without Medulla access.
 * Kept visually distinct (dashed primary border) so the demo can never be
 * mistaken for the real orchestration surface.
 */
export default function DemoScaleBanner({ className }: { className?: string }) {
  const { t } = useT();
  return (
    <div
      role="status"
      data-testid="orch-demo-banner"
      className={`flex items-center gap-2.5 rounded-xl border border-dashed border-primary-300 bg-primary-50/70 px-3.5 py-2.5 text-xs font-medium text-primary-800 dark:border-primary-500/40 dark:bg-primary-500/10 dark:text-primary-200 ${className ?? ''}`}>
      <svg
        className="h-4 w-4 flex-shrink-0 text-primary-500"
        fill="none"
        stroke="currentColor"
        viewBox="0 0 24 24"
        aria-hidden="true">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
        />
      </svg>
      <span>{t('orchPage.demo.banner')}</span>
    </div>
  );
}
