/**
 * Returns the display-safe message carried by an API rejection.
 *
 * `apiClient` rejects with plain `{ success, error }` objects rather than
 * `Error` instances, while other callers may still throw an `Error`. Keep this
 * boundary narrow: only a non-empty string may reach the UI, never the raw
 * rejected payload.
 */
export function messageForApiError(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message.trim()) return error.message;

  if (error && typeof error === 'object' && 'error' in error) {
    const { error: message } = error as { error?: unknown };
    if (typeof message === 'string' && message.trim()) return message;
  }

  return fallback;
}
