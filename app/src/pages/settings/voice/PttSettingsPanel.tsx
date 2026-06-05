/**
 * PttSettingsPanel — settings card for the global push-to-talk hotkey.
 *
 * Renders three controls bound to `pttSlice` (T8):
 *  - A hotkey-capture input that writes the captured key into
 *    `setPttShortcut` (null when cleared). Modifier-only presses are
 *    rejected with an inline error since they don't make sense for PTT.
 *  - A "Speak agent replies" switch bound to `setSpeakReplies`.
 *  - A "Show overlay while held" switch bound to `setShowOverlay`.
 *
 * The hotkey registration side effect itself is handled by
 * `usePttHotkey` (T11) which subscribes to slice changes and forwards
 * to the Tauri shell — this panel only mutates Redux state and lets
 * the manager hook react. This separation keeps the settings UI
 * purely declarative and means the panel test does not need to mock
 * the Tauri command surface.
 *
 * The panel deliberately renders without a `SettingsHeader` since it's
 * intended to be embedded inside `VoicePanel` rather than mounted as a
 * standalone route. The "card" style matches the other sections inside
 * VoicePanel.
 *
 * Plan: docs/superpowers/plans/2026-06-02-global-ptt.md (Task 13).
 */
import { useCallback, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import {
  selectPttRegistrationError,
  selectPttShortcut,
  selectShowOverlay,
  selectSpeakReplies,
  setPttShortcut,
  setShowOverlay,
  setSpeakReplies,
} from '../../../store/pttSlice';

/** Keys that are pure modifiers — a PTT binding made of only these makes
 * no sense (you can't "release" a modifier to send a sample without
 * already needing a non-modifier sentinel). We surface a typed error
 * instead of silently saving a useless binding. */
const MODIFIER_KEYS = new Set([
  'Shift',
  'Control',
  'Alt',
  'Meta',
  'OS',
  'AltGraph',
  'CapsLock',
  'NumLock',
  'ScrollLock',
]);

/**
 * Convert a KeyboardEvent into a stable shortcut string. Mirrors the
 * format the Tauri shell expects (e.g. `Ctrl+Alt+F13`). We use the
 * `key` field (and `code` for letters where `key` carries the layout's
 * uppercased value) to avoid layout drift across QWERTY / AZERTY / etc.
 */
function eventToShortcut(e: React.KeyboardEvent): string | null {
  if (MODIFIER_KEYS.has(e.key)) return null;
  const parts: string[] = [];
  if (e.ctrlKey) parts.push('Ctrl');
  if (e.altKey) parts.push('Alt');
  if (e.shiftKey) parts.push('Shift');
  if (e.metaKey) parts.push('Meta');
  // Prefer e.key (already the localised label like "F13", "a", "Enter")
  // unless it's a single lowercase letter — for those we uppercase to
  // produce a consistent "Ctrl+A" form across capitalised / not.
  // Normalize Space (" ") to the display label "Space" so the saved
  // binding is readable (e.g. "Ctrl+Space" rather than "Ctrl+ ").
  let label = e.key === ' ' ? 'Space' : e.key;
  if (label.length === 1 && /[a-z]/.test(label)) {
    label = label.toUpperCase();
  }
  parts.push(label);
  return parts.join('+');
}

/**
 * Map a raw Tauri error string from `register_ptt_hotkey` to a localized
 * message. Pattern-matches on well-known substrings so the panel doesn't need
 * to depend on the exact Rust error wording; falls back to the raw string for
 * anything unrecognised (still useful to the user for diagnostics).
 */
function localizedRegistrationError(raw: string | null, t: (key: string) => string): string | null {
  if (!raw) return null;
  const lower = raw.toLowerCase();
  if (lower.includes('conflict') && lower.includes('dictation')) {
    return t('pttSettings.errorConflictsWithDictation');
  }
  if (lower.includes('wayland')) {
    return t('pttSettings.errorUnsupportedWayland');
  }
  if (lower.includes('accessibility')) {
    return t('pttSettings.errorAccessibility');
  }
  if (lower.includes('in use') || lower.includes('shortcutinuse') || lower.includes('in_use')) {
    return t('pttSettings.errorShortcutInUse');
  }
  return raw;
}

const PttSettingsPanel = () => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const shortcut = useAppSelector(selectPttShortcut);
  const speakReplies = useAppSelector(selectSpeakReplies);
  const showOverlay = useAppSelector(selectShowOverlay);
  const registrationError = useAppSelector(selectPttRegistrationError);

  // Inline validation error for the capture input (e.g. modifier-only).
  // Cleared whenever the user retries or focuses the field. Server-side
  // errors (accessibility, in-use, Wayland) are emitted by the manager
  // hook via toast/snackbar in T11; we keep this panel-local state for
  // the capture-time failure modes.
  const [captureError, setCaptureError] = useState<string | null>(null);

  const handleShortcutKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLInputElement>) => {
      // Let Tab / Shift+Tab pass through so keyboard navigation within
      // the settings panel still works. All other keys are captured as
      // potential binding candidates and their default actions suppressed
      // so the input doesn't insert text.
      if (e.key === 'Tab') {
        return;
      }
      e.preventDefault();
      e.stopPropagation();

      // Allow Backspace / Delete / Escape to clear the binding so the
      // user can drop back to the "off" state without having to fight a
      // sticky F13.
      if (e.key === 'Backspace' || e.key === 'Delete' || e.key === 'Escape') {
        setCaptureError(null);
        dispatch(setPttShortcut(null));
        return;
      }

      if (MODIFIER_KEYS.has(e.key)) {
        setCaptureError(t('pttSettings.errorModifierOnly'));
        return;
      }

      const shortcutString = eventToShortcut(e);
      if (!shortcutString) {
        setCaptureError(t('pttSettings.errorEmpty'));
        return;
      }

      console.debug('[pttSettings] captured shortcut %s', shortcutString);
      setCaptureError(null);
      dispatch(setPttShortcut(shortcutString));
    },
    [dispatch, t]
  );

  const toggleSpeakReplies = useCallback(() => {
    dispatch(setSpeakReplies(!speakReplies));
  }, [dispatch, speakReplies]);

  const toggleShowOverlay = useCallback(() => {
    dispatch(setShowOverlay(!showOverlay));
  }, [dispatch, showOverlay]);

  return (
    <section className="space-y-3" data-testid="ptt-settings-panel">
      <div className="bg-stone-50 dark:bg-neutral-800/60 rounded-lg border border-stone-200 dark:border-neutral-800 p-4 space-y-4">
        <div>
          <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
            {t('pttSettings.title')}
          </h3>
          <p className="text-xs text-stone-500 dark:text-neutral-400 mt-1">
            {t('pttSettings.description')}
          </p>
        </div>

        {/* Hotkey capture */}
        <label className="block space-y-1">
          <span className="text-xs font-medium text-stone-600 dark:text-neutral-300">
            {t('pttSettings.shortcutLabel')}
          </span>
          <input
            data-testid="ptt-shortcut-input"
            type="text"
            readOnly
            value={shortcut ?? ''}
            placeholder={t('pttSettings.shortcutPlaceholder')}
            aria-label={t('pttSettings.shortcutLabel')}
            onKeyDown={handleShortcutKeyDown}
            onFocus={() => setCaptureError(null)}
            className="w-full rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:outline-none focus:ring-1 focus:ring-primary-400"
          />
          {!shortcut && !captureError && (
            <p
              className="text-[11px] text-stone-500 dark:text-neutral-400 mt-0.5"
              data-testid="ptt-shortcut-unset-hint">
              {t('pttSettings.shortcutUnsetHint')}
            </p>
          )}
          {captureError && (
            <p
              className="text-[11px] text-red-600 dark:text-red-300 mt-0.5"
              data-testid="ptt-shortcut-error">
              {captureError}
            </p>
          )}
          {!captureError && registrationError && (
            <p
              role="alert"
              className="mt-1 text-xs text-red-600 dark:text-red-400"
              data-testid="ptt-registration-error">
              {localizedRegistrationError(registrationError, t)}
            </p>
          )}
        </label>

        {/* Speak replies switch */}
        <div className="flex items-center justify-between">
          <span className="text-xs font-medium text-stone-700 dark:text-neutral-200">
            {t('pttSettings.speakRepliesLabel')}
          </span>
          <button
            type="button"
            role="switch"
            aria-checked={speakReplies}
            data-testid="ptt-speak-replies-switch"
            aria-label={t('pttSettings.speakRepliesLabel')}
            onClick={toggleSpeakReplies}
            className={`relative inline-flex h-4 w-7 shrink-0 items-center rounded-full transition-colors ${
              speakReplies ? 'bg-primary-500' : 'bg-stone-300 dark:bg-neutral-600'
            }`}>
            <span
              aria-hidden
              className={`inline-block h-3 w-3 transform rounded-full bg-white shadow transition-transform ${
                speakReplies ? 'translate-x-3.5' : 'translate-x-0.5'
              }`}
            />
          </button>
        </div>

        {/* Show overlay switch */}
        <div className="flex items-center justify-between">
          <span className="text-xs font-medium text-stone-700 dark:text-neutral-200">
            {t('pttSettings.showOverlayLabel')}
          </span>
          <button
            type="button"
            role="switch"
            aria-checked={showOverlay}
            data-testid="ptt-show-overlay-switch"
            aria-label={t('pttSettings.showOverlayLabel')}
            onClick={toggleShowOverlay}
            className={`relative inline-flex h-4 w-7 shrink-0 items-center rounded-full transition-colors ${
              showOverlay ? 'bg-primary-500' : 'bg-stone-300 dark:bg-neutral-600'
            }`}>
            <span
              aria-hidden
              className={`inline-block h-3 w-3 transform rounded-full bg-white shadow transition-transform ${
                showOverlay ? 'translate-x-3.5' : 'translate-x-0.5'
              }`}
            />
          </button>
        </div>
      </div>
    </section>
  );
};

export default PttSettingsPanel;
