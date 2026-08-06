import debug from 'debug';

import { store } from '../store';
import { notificationReceived } from '../store/notificationSlice';

const log = debug('config-recovery-notice');

/**
 * Stable id so repeated snapshot polls that carry the latched
 * `configRecovered` flag collapse onto a single notification-center row
 * (`notificationReceived` dedupes by id). The once-guard below is the primary
 * defence; this id keeps things idempotent even across it.
 */
const NOTICE_ID = 'config-recovered';

/**
 * i18n keys for the notice copy. English fallbacks are passed alongside so the
 * notice still renders if a locale lacks the key (the `t(key, fallback)`
 * contract). Wording is deliberately accurate for *both* recovery outcomes —
 * the core sets one `configRecovered` flag whether it restored the previous
 * settings from a `.bak` backup or reset to defaults, so the copy must not
 * claim a hard "reset to defaults" that would be wrong in the backup case
 * (#5167).
 */
const TITLE_KEY = 'notifications.configRecovered.title';
const BODY_KEY = 'notifications.configRecovered.body';
const TITLE_FALLBACK = 'Settings file recovered';
const BODY_FALLBACK =
  'Your settings file could not be read, so it was restored from a backup or reset to ' +
  'defaults. The unreadable file was kept with a ".corrupted" suffix in case you need it.';

/** Minimal translator contract — matches `useT()`'s `t(key, fallback?)`. */
type TranslateFn = (key: string, fallback?: string) => string;

/**
 * One-shot guard: the core latches `configRecovered` for the whole process
 * lifetime, so every `app_state_snapshot` poll (~every few seconds) reports it.
 * Without this guard each poll would re-dispatch the notice, resetting it to
 * unread and re-firing the native banner. Surface it exactly once per app run.
 */
let surfaced = false;

/**
 * Raise a single user-visible notice when the core reports it recovered a
 * corrupted `config.toml` this session (#5167). No-op when `configRecovered`
 * is false/absent or the notice was already shown this run.
 *
 * Rendered in the in-app notification center (System category) and, when the
 * window is unfocused, as an OS banner — the same surface as other
 * core-originated system notices.
 *
 * `t` localizes the copy against the active locale; pass it from a hook-aware
 * caller (e.g. `CoreStateProvider` via `useT()`).
 */
export function maybeSurfaceConfigRecovery(
  configRecovered: boolean | undefined,
  t: TranslateFn
): void {
  if (!configRecovered || surfaced) return;
  surfaced = true;
  log('surfacing config-recovery notice');
  store.dispatch(
    notificationReceived({
      id: NOTICE_ID,
      category: 'system',
      title: t(TITLE_KEY, TITLE_FALLBACK),
      body: t(BODY_KEY, BODY_FALLBACK),
      timestamp: Date.now(),
      read: false,
      deepLink: '/settings',
    })
  );
}

/** Test-only: reset the once-guard between runs. */
export function __resetForTests(): void {
  surfaced = false;
}
