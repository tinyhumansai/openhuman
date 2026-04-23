import debugFactory from 'debug';

import { SHARE_BASE_URL } from './config';
import { openUrl } from './openUrl';

const log = debugFactory('share');

export type SharePlatform =
  | 'twitter'
  | 'telegram'
  | 'whatsapp'
  | 'linkedin'
  | 'facebook'
  | 'reddit'
  | 'email'
  | 'sms';

/**
 * Build a public invite URL that friends can click from anywhere.
 *
 * Routes to the web onboarding flow with `?invite=CODE`; the desktop/mobile
 * app also handles `openhuman://invite/CODE` for deep linking.
 */
export function buildInviteUrl(code: string): string {
  const trimmed = code.trim();
  if (!trimmed) return SHARE_BASE_URL;
  return `${SHARE_BASE_URL}/i/${encodeURIComponent(trimmed)}`;
}

/** Short, social-optimized default copy for invite posts. */
export function defaultInviteMessage(code: string, url: string): string {
  return `I'm using OpenHuman — a private AI super-assistant that actually gets me. Join with my code ${code}: ${url}`;
}

const SHARE_URL_BUILDERS: Record<SharePlatform, (text: string, url: string) => string> = {
  twitter: (text, url) =>
    `https://twitter.com/intent/tweet?text=${encodeURIComponent(text)}&url=${encodeURIComponent(url)}`,
  telegram: (text, url) =>
    `https://t.me/share/url?url=${encodeURIComponent(url)}&text=${encodeURIComponent(text)}`,
  whatsapp: (text, url) => `https://wa.me/?text=${encodeURIComponent(`${text} ${url}`)}`,
  linkedin: (_text, url) =>
    `https://www.linkedin.com/sharing/share-offsite/?url=${encodeURIComponent(url)}`,
  facebook: (_text, url) =>
    `https://www.facebook.com/sharer/sharer.php?u=${encodeURIComponent(url)}`,
  reddit: (text, url) =>
    `https://www.reddit.com/submit?title=${encodeURIComponent(text)}&url=${encodeURIComponent(url)}`,
  email: (text, url) =>
    `mailto:?subject=${encodeURIComponent('Try OpenHuman with me')}&body=${encodeURIComponent(
      `${text}\n\n${url}`
    )}`,
  sms: (text, url) => `sms:?&body=${encodeURIComponent(`${text} ${url}`)}`,
};

/** Construct the platform-specific share URL for a given message + target URL. */
export function buildShareUrl(platform: SharePlatform, text: string, url: string): string {
  return SHARE_URL_BUILDERS[platform](text, url);
}

/** Open the share intent for the given platform in the default browser / app. */
export async function shareOn(platform: SharePlatform, text: string, url: string): Promise<void> {
  const target = buildShareUrl(platform, text, url);
  log('opening share intent', { platform });
  await openUrl(target);
}

/**
 * Copy text to the clipboard. Returns `true` on success.
 *
 * Uses the async Clipboard API when available; falls back to a hidden
 * textarea + `document.execCommand` for older surfaces (e.g. webviews
 * without navigator.clipboard).
 */
export async function copyToClipboard(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch (error) {
    log('navigator.clipboard.writeText failed', error);
  }

  try {
    const textarea = document.createElement('textarea');
    textarea.value = text;
    textarea.setAttribute('readonly', '');
    textarea.style.position = 'fixed';
    textarea.style.opacity = '0';
    document.body.appendChild(textarea);
    textarea.select();
    const ok = document.execCommand('copy');
    document.body.removeChild(textarea);
    return ok;
  } catch (error) {
    log('textarea fallback failed', error);
    return false;
  }
}

/**
 * Invoke the native OS share sheet when supported (mobile browsers, modern
 * desktop). Returns `true` if the share sheet was shown (or accepted), or
 * `false` if the platform lacks support so the caller can show a fallback UI.
 */
export async function tryNativeShare(payload: {
  title?: string;
  text?: string;
  url?: string;
}): Promise<boolean> {
  if (typeof navigator === 'undefined' || typeof navigator.share !== 'function') {
    return false;
  }
  try {
    await navigator.share(payload);
    log('native share succeeded');
    return true;
  } catch (error) {
    if ((error as Error)?.name === 'AbortError') {
      log('native share cancelled by user');
      return true;
    }
    log('native share failed', error);
    return false;
  }
}
