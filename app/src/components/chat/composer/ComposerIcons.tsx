import { LuUserRound } from 'react-icons/lu';

import { COMPOSER_CANCEL_ICON, COMPOSER_SEND_ICON } from './composerStyles';

/** Plus glyph for the add-attachment control (`.aui-composer-add-attachment`). */
export function AddAttachmentIcon() {
  return (
    <svg
      className="size-5"
      fill="none"
      stroke="currentColor"
      viewBox="0 0 24 24"
      aria-hidden="true">
      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.8} d="M12 5v14m-7-7h14" />
    </svg>
  );
}

/** Microphone glyph for the voice-mode control. No upstream equivalent. */
export function MicIcon() {
  return (
    <svg
      className="size-4"
      fill="none"
      stroke="currentColor"
      viewBox="0 0 24 24"
      aria-hidden="true">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M12 1a3 3 0 00-3 3v8a3 3 0 006 0V4a3 3 0 00-3-3z"
      />
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M19 10v2a7 7 0 01-14 0v-2M12 19v4m-4 0h8"
      />
    </svg>
  );
}

/**
 * Person glyph for the composer's idle primary action, which opens the Human
 * page. Deliberately the same lucide `LuUserRound` the sidebar's Human tab
 * uses (`shell/navIcons.tsx`) so the button reads as *that destination* rather
 * than as a second, unrelated voice control beside the mic.
 */
export function HumanModeIcon() {
  return <LuUserRound className={COMPOSER_SEND_ICON} aria-hidden="true" />;
}

/** Send arrow, sized by `.aui-composer-send-icon`. */
export function SendIcon() {
  return (
    <svg
      className={COMPOSER_SEND_ICON}
      fill="none"
      stroke="currentColor"
      viewBox="0 0 24 24"
      aria-hidden="true">
      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2.5} d="M9 5l7 7-7 7" />
    </svg>
  );
}

/** Stop square, sized by `.aui-composer-cancel-icon` (`size-3 fill-current`). */
export function StopIcon() {
  return (
    <svg className={COMPOSER_CANCEL_ICON} viewBox="0 0 24 24" aria-hidden="true">
      <rect x="6" y="6" width="12" height="12" rx="1.5" />
    </svg>
  );
}

/** In-flight spinner shown in the send slot for a normal (non-parallel) send. */
export function SendingSpinnerIcon() {
  return (
    <svg
      className={`${COMPOSER_SEND_ICON} animate-spin`}
      fill="none"
      viewBox="0 0 24 24"
      aria-hidden="true">
      <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
      <path
        className="opacity-75"
        fill="currentColor"
        d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
      />
    </svg>
  );
}
