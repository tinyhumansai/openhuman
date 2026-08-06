import { useCallback, useEffect, useRef, useState } from 'react';

export type ClipboardFeedbackStatus = 'idle' | 'copied' | 'error';

export interface UseClipboardFeedbackOptions {
  resetAfterMs?: number;
  writeText?: (value: string) => Promise<void>;
}

export interface UseClipboardFeedbackResult {
  status: ClipboardFeedbackStatus;
  copy: (value: string) => Promise<boolean>;
  reset: () => void;
}

const DEFAULT_RESET_AFTER_MS = 2000;

export function useClipboardFeedback(
  options: UseClipboardFeedbackOptions = {}
): UseClipboardFeedbackResult {
  const [status, setStatus] = useState<ClipboardFeedbackStatus>('idle');
  const mountedRef = useRef(true);
  const operationIdRef = useRef(0);
  const resetTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const writeTextRef = useRef(options.writeText);
  const resetAfterMsRef = useRef(options.resetAfterMs ?? DEFAULT_RESET_AFTER_MS);

  writeTextRef.current = options.writeText;
  resetAfterMsRef.current = options.resetAfterMs ?? DEFAULT_RESET_AFTER_MS;

  const clearResetTimer = useCallback(() => {
    if (resetTimerRef.current !== null) {
      clearTimeout(resetTimerRef.current);
      resetTimerRef.current = null;
    }
  }, []);

  const scheduleFeedbackReset = useCallback(
    (operationId: number) => {
      clearResetTimer();
      resetTimerRef.current = setTimeout(() => {
        if (mountedRef.current && operationId === operationIdRef.current) {
          resetTimerRef.current = null;
          setStatus('idle');
        }
      }, resetAfterMsRef.current);
    },
    [clearResetTimer]
  );

  const reset = useCallback(() => {
    operationIdRef.current += 1;
    clearResetTimer();
    if (mountedRef.current) {
      setStatus('idle');
    }
  }, [clearResetTimer]);

  const copy = useCallback(
    async (value: string): Promise<boolean> => {
      const operationId = ++operationIdRef.current;
      clearResetTimer();

      try {
        const writeText =
          writeTextRef.current ?? ((nextValue: string) => navigator.clipboard.writeText(nextValue));
        await writeText(value);

        if (mountedRef.current && operationId === operationIdRef.current) {
          setStatus('copied');
          scheduleFeedbackReset(operationId);
        }
        return true;
      } catch {
        if (mountedRef.current && operationId === operationIdRef.current) {
          setStatus('error');
          scheduleFeedbackReset(operationId);
        }
        return false;
      }
    },
    [clearResetTimer, scheduleFeedbackReset]
  );

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      operationIdRef.current += 1;
      clearResetTimer();
    };
  }, [clearResetTimer]);

  return { status, copy, reset };
}
