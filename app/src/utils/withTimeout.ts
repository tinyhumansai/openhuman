import { TOOL_TIMEOUT_SECS } from './config';

/**
 * Reject with a clear error if `promise` does not settle within `timeoutMs`.
 * Clears the timer when the promise completes.
 */
export async function withTimeout<T>(
  promise: Promise<T>,
  timeoutMs: number,
  label: string
): Promise<T> {
  if (timeoutMs <= 0) return promise;

  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeoutPromise = new Promise<never>((_, reject) => {
    timer = setTimeout(() => {
      reject(new Error(`${label} timed out after ${Math.round(timeoutMs / 1000)}s`));
    }, timeoutMs);
  });

  try {
    return await Promise.race([promise, timeoutPromise]);
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

/** Default matches core `OPENHUMAN_TOOL_TIMEOUT_SECS` (120). */
export function toolExecutionTimeoutMsFromEnv(): number {
  return Math.round(TOOL_TIMEOUT_SECS * 1000);
}
