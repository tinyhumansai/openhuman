import { useT } from '../../../../lib/i18n/I18nContext';

/** Placeholder bubbles shown while a thread's messages are being fetched. */
export function TranscriptSkeleton() {
  return (
    <div className="mx-auto w-full max-w-195 space-y-4 px-5 py-4">
      {Array.from({ length: 4 }).map((_, i) => (
        <div key={i} className={`flex ${i % 2 === 0 ? 'justify-start' : 'justify-end'}`}>
          <div
            className={`h-12 rounded-2xl animate-pulse bg-surface-subtle ${
              i % 2 === 0 ? 'w-2/3' : 'w-1/2'
            }`}
          />
        </div>
      ))}
    </div>
  );
}

/** Terminal state for a thread whose message history failed to load. */
export function TranscriptLoadError({ message }: { message: string }) {
  const { t } = useT();
  return (
    <div className="flex-1 flex flex-col items-center justify-center h-full">
      <svg
        className="w-8 h-8 text-coral-500/70 mb-3"
        fill="none"
        stroke="currentColor"
        viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={1.5}
          d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
        />
      </svg>
      <p className="text-sm text-content-faint mb-1">{t('chat.failedToLoadMessages')}</p>
      <p className="text-xs text-content-secondary mb-3 text-center">{message}</p>
      <button
        type="button"
        data-analytics-id="chat-messages-reload"
        onClick={() => window.location.reload()}
        className="text-xs text-primary-400 hover:text-primary-300 transition-colors">
        {t('common.reload')}
      </button>
    </div>
  );
}
