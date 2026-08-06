import { useEffect, useState } from 'react';

export function useDebouncedValue<T>(value: T, delayMs: number): T {
  const [debouncedValue, setDebouncedValue] = useState<T>(() => value);
  const normalizedDelayMs = Math.max(0, delayMs);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      setDebouncedValue(() => value);
    }, normalizedDelayMs);

    return () => window.clearTimeout(timer);
  }, [value, normalizedDelayMs]);

  return debouncedValue;
}
