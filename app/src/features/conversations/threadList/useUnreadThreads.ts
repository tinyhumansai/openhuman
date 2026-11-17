import { useEffect, useRef, useState } from 'react';

/**
 * Tracks which threads finished a reply while the user was looking elsewhere.
 *
 * A thread becomes unread on the running → idle edge when it is not the
 * selected thread, and is cleared the moment it is selected. Runtime-only:
 * the core has no read receipts, so this resets on reload rather than
 * inventing a persisted "seen" state the backend cannot confirm.
 *
 * The ids are folded into a string key, so a fresh array with the same
 * content each render does not re-run the edge detection.
 */
const SEPARATOR = '\u0000';

export function useUnreadThreads(
  runningThreadIds: readonly string[],
  selectedThreadId: string | null
): ReadonlySet<string> {
  const [unread, setUnread] = useState<ReadonlySet<string>>(() => new Set());
  const previousRunning = useRef<ReadonlySet<string>>(new Set());
  const runningKey = runningThreadIds.join(SEPARATOR);

  useEffect(() => {
    const running = new Set(runningKey ? runningKey.split(SEPARATOR) : []);
    const finished = [...previousRunning.current].filter(
      id => !running.has(id) && id !== selectedThreadId
    );
    previousRunning.current = running;
    if (finished.length === 0) return;
    setUnread(prev => {
      const next = new Set(prev);
      for (const id of finished) next.add(id);
      return next;
    });
  }, [runningKey, selectedThreadId]);

  useEffect(() => {
    if (!selectedThreadId) return;
    setUnread(prev => {
      if (!prev.has(selectedThreadId)) return prev;
      const next = new Set(prev);
      next.delete(selectedThreadId);
      return next;
    });
  }, [selectedThreadId]);

  return unread;
}
