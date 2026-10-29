import * as Sentry from '@sentry/react';
import type { ReactNode } from 'react';
import { createHashRouter } from 'react-router-dom';

import ErrorFallbackScreen from '../../components/ErrorFallbackScreen';

/**
 * The desktop shell needs a data-router-aware blocker while retaining hash
 * URLs for installed desktop/mobile applications. All routes are still owned
 * by the existing AppRoutes component rendered within this root shell.
 *
 * React Router catches route render errors before the outer App-level Sentry
 * boundary. Keep the application's Sentry-backed recovery screen here too, so
 * enabling data-router navigation does not regress crash reporting or recovery.
 */
export function createAppHashRouter(shell: ReactNode) {
  return createHashRouter([
    {
      path: '*',
      element: (
        <Sentry.ErrorBoundary
          fallback={({ error, componentStack, resetError, eventId }) => (
            <ErrorFallbackScreen
              error={error}
              componentStack={componentStack}
              eventId={eventId}
              onReset={resetError}
            />
          )}>
          {shell}
        </Sentry.ErrorBoundary>
      ),
    },
  ]);
}
