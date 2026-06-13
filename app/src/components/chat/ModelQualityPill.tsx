import { useT } from '../../lib/i18n/I18nContext';

interface ModelQualityPillProps {
  className?: string;
}

/**
 * Compact read-only pill that shows the current model name and quality tier
 * in the chat composer toolbar. The chevron is decorative (v1: no dropdown).
 */
export default function ModelQualityPill({ className }: ModelQualityPillProps) {
  const { t } = useT();

  return (
    <button
      type="button"
      data-analytics-id="chat-model-quality-pill"
      aria-label={t('composer.modelSelector')}
      title={t('composer.modelSelector')}
      disabled
      className={`flex items-center gap-1 text-xs text-stone-400 dark:text-neutral-500 disabled:cursor-default disabled:opacity-100 select-none ${className ?? ''}`}>
      <span>OpenHuman</span>
      <span className="text-stone-300 dark:text-neutral-600">·</span>
      <span>{t('composer.qualityHigh')}</span>
      <svg
        className="w-3 h-3 ml-0.5"
        fill="none"
        stroke="currentColor"
        viewBox="0 0 24 24"
        aria-hidden>
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
      </svg>
    </button>
  );
}
