/**
 * useLocalModelStatus — returns true when a local AI model is ready and serving.
 *
 * Polls openhumanLocalAiStatus() every POLL_INTERVAL_MS.
 * Returns false in non-Tauri environments (browser dev / remote-only).
 * This is the gate used by the multi-bubble delivery path.
 */
import { useEffect, useRef, useState } from 'react';

import { isTauri, openhumanLocalAiStatus } from '../utils/tauriCommands';

const POLL_INTERVAL_MS = 12_000;

export function useLocalModelStatus(): boolean {
  const [isActive, setIsActive] = useState(false);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;

    if (!isTauri()) return;

    const check = async () => {
      try {
        const res = await openhumanLocalAiStatus();
        if (mountedRef.current) {
          setIsActive(res.result?.state === 'ready');
        }
      } catch {
        if (mountedRef.current) setIsActive(false);
      }
    };

    void check();
    const id = setInterval(() => void check(), POLL_INTERVAL_MS);

    return () => {
      mountedRef.current = false;
      clearInterval(id);
    };
  }, []);

  return isActive;
}
