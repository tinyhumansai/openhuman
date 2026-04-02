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
  const raw = import.meta.env.VITE_TOOL_TIMEOUT_SECS as string | undefined;
  if (raw === undefined || raw === '') return 120_000;
  const n = Number(raw);
  if (!Number.isFinite(n) || n <= 0 || n > 3600) return 120_000;
  return Math.round(n * 1000);
}
