import { useCallback, useEffect, useRef } from 'react';

export interface LatestAsyncGuard {
  begin: () => number;
  isLatest: (generation: number) => boolean;
  invalidate: () => void;
}

export function useLatestAsync(): LatestAsyncGuard {
  const generationRef = useRef(0);
  const mountedRef = useRef(true);

  const begin = useCallback(() => {
    generationRef.current += 1;
    return generationRef.current;
  }, []);

  const isLatest = useCallback(
    (generation: number) => mountedRef.current && generation === generationRef.current,
    []
  );

  const invalidate = useCallback(() => {
    generationRef.current += 1;
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      invalidate();
    };
  }, [invalidate]);

  return { begin, isLatest, invalidate };
}
