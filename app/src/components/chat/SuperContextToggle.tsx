import debugFactory from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { isTauri } from '../../utils/tauriCommands/common';
import {
  openhumanGetSuperContextEnabled,
  openhumanSetSuperContextEnabled,
} from '../../utils/tauriCommands/config';
import SettingsSwitch from '../settings/controls/SettingsSwitch';
import { trackSuperContextWrite } from './superContextWrite';

const log = debugFactory('chat:super-context-toggle');

/**
 * "Super context" toggle, rendered directly below the chat composer.
 *
 * Flips the persistent `context.super_context_enabled` core config flag. When
 * on, the harness runs a read-only context-collection pass on the **first turn
 * of a new thread** — before the orchestrator LLM runs — and folds the result
 * into the user message. This is harness-driven (deterministic), unlike the
 * `agent_prepare_context` tool the model may call on its own.
 *
 * Because the flag is read at thread construction, toggling it only affects
 * threads started afterwards — surfaced to the user via the helper hint.
 */
const SuperContextToggle = () => {
  const { t } = useT();
  const [enabled, setEnabled] = useState(false);
  // Until the first read resolves we don't know the real value; keep the
  // switch disabled so a stray click can't write a stale default back. Outside
  // Tauri (Storybook/web preview) there's no core to read, so treat the control
  // as loaded immediately in its default-off state without hitting the RPC.
  const [loaded, setLoaded] = useState(() => !isTauri());
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!isTauri()) {
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const res = await openhumanGetSuperContextEnabled();
        if (!cancelled) {
          setEnabled(Boolean(res.result));
          log('loaded super_context_enabled=%o', res.result);
        }
      } catch (err) {
        // Best-effort: a read failure leaves the toggle in its default-off
        // state. The user can still flip it; the write path surfaces errors.
        log('failed to load super_context_enabled: %o', err);
      } finally {
        if (!cancelled) setLoaded(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const handleChange = useCallback(
    (next: boolean) => {
      if (busy) return;
      const previous = enabled;
      // Optimistic: reflect the choice immediately, roll back on failure.
      setEnabled(next);
      setBusy(true);
      log('set super_context_enabled -> %o', next);
      const write = openhumanSetSuperContextEnabled(next);
      // Register the write so a flip-then-immediately-Send awaits it before the
      // new thread's session reads the persisted flag (avoids a stale first turn).
      trackSuperContextWrite(write);
      void (async () => {
        try {
          const res = await write;
          setEnabled(Boolean(res.result));
        } catch (err) {
          log('failed to persist super_context_enabled, rolling back: %o', err);
          setEnabled(previous);
        } finally {
          setBusy(false);
        }
      })();
    },
    [busy, enabled]
  );

  return (
    <div className="flex h-7 flex-shrink-0 items-center gap-1.5 text-xs text-content-muted">
      <SettingsSwitch
        id="super-context-toggle"
        checked={enabled}
        onCheckedChange={handleChange}
        disabled={!loaded || busy}
        aria-label={t('chat.superContext.label')}
        data-testid="super-context-toggle"
      />
      <span className="font-medium text-content-secondary">{t('chat.superContext.label')}</span>
      {/* Self-contained wrapping tooltip (the shared <Tooltip> is single-line
          nowrap and can't fit this paragraph). Anchored bottom-full + right-0
          so it grows up-and-left into the app interior — the toggle only shows
          on a fresh thread, where that space is empty, so it never clips. */}
      <span className="group relative inline-flex">
        <button
          type="button"
          aria-describedby="super-context-tooltip"
          aria-label={t('chat.superContext.label')}
          data-testid="super-context-info"
          className="flex h-4 w-4 items-center justify-center rounded-full text-content-faint transition-colors hover:text-content-secondary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary-500">
          <svg
            className="h-3.5 w-3.5"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
            aria-hidden="true">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={1.8}
              d="M12 16v-4m0-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
            />
          </svg>
        </button>
        <span
          id="super-context-tooltip"
          role="tooltip"
          className="pointer-events-none absolute bottom-full right-0 z-[9999] mb-2 w-72 rounded-lg bg-stone-800 px-3 py-2 text-xs font-normal leading-snug text-white opacity-0 shadow-lg transition-opacity duration-150 group-hover:opacity-100 group-focus-within:opacity-100 dark:bg-neutral-700">
          {t('chat.superContext.hint')}
        </span>
      </span>
    </div>
  );
};

export default SuperContextToggle;
