/**
 * ErrorFallbackScreen
 *
 * Full-screen recovery UI shown when the Sentry ErrorBoundary catches
 * a catastrophic React render error. Self-contained with zero dependencies
 * on Redux, Router, or any context provider.
 *
 * The ErrorReportNotification lives in a separate React root, so the user
 * can still review and report the error from this screen.
 */

interface ErrorFallbackScreenProps {
  error: unknown;
  componentStack?: string;
  onReset: () => void;
}

export default function ErrorFallbackScreen({
  error,
  componentStack,
  onReset,
}: ErrorFallbackScreenProps) {
  const errorName = error instanceof Error ? error.name : 'Error';
  const errorMessage = error instanceof Error ? error.message : String(error);

  return (
    <div className="fixed inset-0 flex items-center justify-center bg-gradient-to-b from-stone-950 to-stone-900">
      <div className="w-full max-w-lg mx-4 bg-stone-900 border border-coral-500/30 rounded-2xl shadow-large overflow-hidden">
        {/* Accent bar */}
        <div className="h-1 bg-coral-500" />

        <div className="p-8">
          {/* Icon */}
          <div className="flex justify-center mb-6">
            <div className="w-16 h-16 rounded-full bg-coral-500/10 flex items-center justify-center">
              <svg
                className="w-8 h-8 text-coral-500"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth={1.5}>
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126zM12 15.75h.007v.008H12v-.008z"
                />
              </svg>
            </div>
          </div>

          {/* Title */}
          <h1 className="text-xl font-semibold text-white text-center mb-2">
            Something went wrong
          </h1>
          <p className="text-sm text-stone-400 text-center mb-6">
            The application encountered an unexpected error and could not recover.
          </p>

          {/* Error details */}
          <div className="bg-stone-800/50 border border-stone-700/50 rounded-xl p-4 mb-6">
            <p className="text-sm font-medium text-coral-400 mb-1">{errorName}</p>
            <p className="text-xs text-stone-300 break-words">{errorMessage}</p>
            {componentStack && (
              <details className="mt-3">
                <summary className="text-xs text-stone-500 cursor-pointer hover:text-stone-300 transition-colors">
                  Component stack
                </summary>
                <pre className="mt-2 text-[11px] text-stone-500 whitespace-pre-wrap break-words max-h-[200px] overflow-auto">
                  {componentStack}
                </pre>
              </details>
            )}
          </div>

          {/* Actions */}
          <div className="flex gap-3">
            <button
              onClick={onReset}
              className="flex-1 bg-stone-700 hover:bg-stone-600 text-white text-sm font-medium rounded-xl px-4 py-3 transition-colors">
              Try to Recover
            </button>
            <button
              onClick={() => { window.location.hash = '#/home'; window.location.reload(); }}
              className="flex-1 bg-coral-500 hover:bg-coral-600 text-white text-sm font-medium rounded-xl px-4 py-3 transition-colors">
              Reload App
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
