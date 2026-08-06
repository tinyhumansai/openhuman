/**
 * Pure builders for the social share-intent URLs used by the share cards
 * feature (#5006). Kept dependency-free so they can be unit-tested and reused
 * from the modal without pulling in React or Tauri.
 *
 * Neither X nor LinkedIn lets a web intent attach a raw image blob, so the flow
 * is: the user copies/saves the generated PNG, then the intent opens a
 * pre-filled composer they drop the image into. The URL carries the caption and
 * a link back to tinyhumans.ai.
 */
import { TWEET_MAX } from './shareContent';

/** Canonical marketing URL shared on every card. */
export const SHARE_LANDING_URL = 'https://tinyhumans.ai';

/**
 * Builds an X (Twitter) web-intent URL that opens the composer pre-filled with
 * `text` and a link to `url`. X counts the (shortened) URL against the 280-char
 * limit as a fixed ~23 chars, so we trim the caption to leave room.
 */
export function buildTweetIntentUrl(text: string, url: string = SHARE_LANDING_URL): string {
  const URL_WEIGHT = 24; // t.co shortener length + a space.
  const room = Math.max(0, TWEET_MAX - URL_WEIGHT);
  const body = text.length > room ? `${text.slice(0, room - 1).trimEnd()}…` : text;
  const params = new URLSearchParams({ text: body, url });
  return `https://twitter.com/intent/tweet?${params.toString()}`;
}

/**
 * Builds a LinkedIn share URL. LinkedIn's public share endpoint
 * (`share-offsite`) only accepts a `url`; it ignores any caption/summary
 * parameter (the old `mini`/`summary` params were removed). The caller is
 * therefore expected to copy the caption to the clipboard and prompt the user
 * to paste it — see `ShareCardModal`.
 */
export function buildLinkedInShareUrl(url: string = SHARE_LANDING_URL): string {
  const params = new URLSearchParams({ url });
  return `https://www.linkedin.com/sharing/share-offsite/?${params.toString()}`;
}
