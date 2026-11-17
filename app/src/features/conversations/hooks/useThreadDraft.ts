/**
 * The composer draft, kept per thread and across reloads (after OpenClaw's
 * per-session composer persistence).
 *
 * Switching threads swaps the draft instead of carrying the half-typed text
 * into the next conversation, and the text survives a reload. Persistence goes
 * through `userScopedStorage`, so one account never sees another's drafts, and
 * is debounced so a keystroke burst costs one write. An empty draft removes
 * its key, which is how a send clears it.
 */
import debugFactory from 'debug';
import { type SetStateAction, useCallback, useEffect, useRef, useState } from 'react';

import { userScopedStorage } from '../../../store/userScopedStorage';

const debug = debugFactory('conversations:draft');

/** Debounce between the last keystroke and the storage write. */
export const DRAFT_SAVE_DEBOUNCE_MS = 200;

export function draftStorageKey(threadId: string): string {
  return `chat:draft:${threadId}`;
}

function persistDraft(threadId: string, text: string): void {
  const key = draftStorageKey(threadId);
  // Lengths only — never the draft text itself.
  debug('[chat][draft] persist thread=%s len=%d', threadId, text.length);
  void (text.length > 0 ? userScopedStorage.setItem(key, text) : userScopedStorage.removeItem(key));
}

interface DraftState {
  threadId: string | null;
  text: string;
  /** Set by an edit; a load or the initial state never writes back. */
  edited: boolean;
}

/**
 * `[draft, setDraft]` for `threadId`, a drop-in for `useState('')`: the setter
 * accepts a value or an updater.
 */
export function useThreadDraft(
  threadId: string | null
): [string, (action: SetStateAction<string>) => void] {
  const [state, setState] = useState<DraftState>({ threadId, text: '', edited: false });
  const threadIdRef = useRef(threadId);
  threadIdRef.current = threadId;
  // The newest edit not yet written, so a thread switch or unmount inside the
  // debounce window still saves it.
  const pendingRef = useRef<DraftState | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const flush = useCallback(() => {
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    const pending = pendingRef.current;
    pendingRef.current = null;
    if (pending?.threadId) persistDraft(pending.threadId, pending.text);
  }, []);

  // Load the stored draft for each thread the view switches to.
  useEffect(() => {
    flush();
    if (!threadId) return;
    let cancelled = false;
    void userScopedStorage.getItem(draftStorageKey(threadId)).then(stored => {
      if (cancelled || !stored) return;
      setState(prev => {
        // Typing that landed before the read finished wins over the stored copy.
        if (prev.threadId === threadId && prev.text.length > 0) return prev;
        debug('[chat][draft] restored thread=%s len=%d', threadId, stored.length);
        return { threadId, text: stored, edited: false };
      });
    });
    return () => {
      cancelled = true;
    };
  }, [flush, threadId]);

  useEffect(() => flush, [flush]);

  const setDraft = useCallback((action: SetStateAction<string>) => {
    const current = threadIdRef.current;
    setState(prev => {
      const base = prev.threadId === current ? prev.text : '';
      const text = typeof action === 'function' ? action(base) : action;
      return text === base && prev.threadId === current
        ? prev
        : { threadId: current, text, edited: true };
    });
  }, []);

  // Schedule the write for whatever the draft became.
  useEffect(() => {
    if (!state.edited || !state.threadId || state.threadId !== threadIdRef.current) return;
    pendingRef.current = state;
    if (timerRef.current) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => {
      timerRef.current = null;
      const pending = pendingRef.current;
      pendingRef.current = null;
      if (pending?.threadId) persistDraft(pending.threadId, pending.text);
    }, DRAFT_SAVE_DEBOUNCE_MS);
  }, [state]);

  return [state.threadId === threadId ? state.text : '', setDraft];
}
