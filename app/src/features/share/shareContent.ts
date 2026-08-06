/**
 * Pure content helpers for the in-app share cards feature (#5006).
 *
 * These functions turn an agent's chat output into the short, on-brand text
 * that appears on the share card and in the pre-filled social caption. They are
 * deliberately dependency-free and synchronous so they can be unit-tested
 * without a running core, a canvas, or the network — the async LLM draft lives
 * in `shareCaption.ts` and delegates its fallback here.
 *
 * PRIVACY (issue #5006 acceptance criterion): nothing private must leak into a
 * shared card. The source text is the assistant message the user already sees,
 * but the agent may have echoed a file path, an API key, or an email address
 * into that reply. `redactSensitive` scrubs those classes before any text is
 * placed on the card or in the caption. Callers must run derived text through
 * it (both `buildFallbackHeadline` and `sanitizeHeadline` already do).
 */

/** Max characters for the headline rendered on the card / seeded into the caption. */
export const SHARE_HEADLINE_MAX = 90;

/** X (Twitter) hard limit for the tweet body. */
export const TWEET_MAX = 280;

const LOG_PREFIX = '[share-content]';

/**
 * Patterns for text that must never appear on a public card. Order matters:
 * broad secret tokens first, then paths, then emails. Each is replaced with a
 * neutral placeholder rather than removed so the surrounding sentence still
 * reads naturally.
 */
const REDACTIONS: ReadonlyArray<{ re: RegExp; with: string }> = [
  // OpenAI / Anthropic / generic provider keys: sk-..., sk-ant-..., etc.
  { re: /\bsk-[a-zA-Z0-9-]{16,}\b/g, with: '[redacted]' },
  // Bearer / auth tokens explicitly labelled.
  { re: /\bBearer\s+[A-Za-z0-9._-]{12,}\b/gi, with: '[redacted]' },
  // AWS access key ids.
  { re: /\bAKIA[0-9A-Z]{16}\b/g, with: '[redacted]' },
  // Long opaque hex runs (>= 32 chars) that look like secrets, not prose.
  { re: /\b[A-Fa-f0-9]{32,}\b/g, with: '[redacted]' },
  // Long opaque base64/base64url runs (>= 24 chars) that mix upper/lower/digit,
  // e.g. an unlabelled webhook secret, JWT-like value, or provider key. This is
  // intentionally last-resort and biased toward over-redaction: a stray
  // camelCase identifier can trip it, but leaking a real secret onto a public
  // card is the worse outcome. Requiring all three character classes keeps it
  // from firing on plain hex (already caught above) or all-caps constants.
  {
    re: /\b(?=[A-Za-z0-9+/_-]{24,}={0,2}\b)(?=[A-Za-z0-9+/_-]*[A-Z])(?=[A-Za-z0-9+/_-]*[a-z])(?=[A-Za-z0-9+/_-]*[0-9])[A-Za-z0-9+/_-]{24,}={0,2}\b/g,
    with: '[redacted]',
  },
  // POSIX absolute paths under common home/system/workspace/application roots.
  // The optional trailing group covers a single space-separated filename token
  // (e.g. "private plan.txt") without swallowing the rest of the sentence: it
  // only fires when that token itself looks like a filename (has a dot
  // extension), so normal prose following the path is left untouched.
  {
    re: /\/(?:Users|home|root|var|etc|tmp|private|workspace|opt|srv|app|data|mnt|Applications|Volumes)\/(?:[^\s"'`)/]+\/)*[^\s"'`)]+(?:[ \t][^\s"'`)]*\.[A-Za-z0-9]{1,8})?/g,
    with: '[path]',
  },
  // Windows absolute paths and UNC shares.
  { re: /[A-Za-z]:\\[^\s"'`)]+/g, with: '[path]' },
  { re: /\\\\[^\s"'`)]+/g, with: '[path]' },
  // Email addresses (PII).
  { re: /\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b/g, with: '[email]' },
];

/**
 * Removes secrets, absolute file paths, and emails from `text`. Idempotent and
 * safe to call on already-clean text. See the module-level PRIVACY note.
 */
export function redactSensitive(text: string): string {
  let out = text;
  for (const { re, with: replacement } of REDACTIONS) {
    out = out.replace(re, replacement);
  }
  return out;
}

/**
 * Strips the common markdown syntax an agent reply carries so the card shows
 * clean prose, not raw `**bold**` / `# headings` / ``` fences. Not a full
 * markdown parser — just enough to make a one-line headline readable.
 */
export function stripMarkdown(text: string): string {
  return (
    text
      // Fenced code blocks -> drop entirely (never headline-worthy).
      .replace(/```[\s\S]*?```/g, ' ')
      // Inline code -> keep the inner text.
      .replace(/`([^`]+)`/g, '$1')
      // Images -> alt text.
      .replace(/!\[([^\]]*)\]\([^)]*\)/g, '$1')
      // Links -> label only.
      .replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
      // Headings / blockquote / list markers at line start.
      .replace(/^\s{0,3}(?:#{1,6}|>|[-*+]|\d+\.)\s+/gm, '')
      // Emphasis markers.
      .replace(/(\*\*|__|\*|_|~~)/g, '')
      // Collapse whitespace.
      .replace(/\s+/g, ' ')
      .trim()
  );
}

/**
 * Truncates `text` to at most `max` characters on a word boundary, appending an
 * ellipsis when it had to cut. Never returns a string longer than `max`.
 */
export function truncateAtWord(text: string, max: number): string {
  const trimmed = text.trim();
  if (trimmed.length <= max) return trimmed;
  // Reserve one character for the ellipsis.
  const slice = trimmed.slice(0, max - 1);
  const lastSpace = slice.lastIndexOf(' ');
  const body = lastSpace > max * 0.5 ? slice.slice(0, lastSpace) : slice;
  return `${body.trimEnd()}…`;
}

/**
 * Builds a deterministic card headline from raw agent output. Used both as the
 * offline fallback when the LLM draft is unavailable and as the input the LLM
 * draft is sanitised against. Returns an empty string if nothing usable remains
 * after cleaning.
 */
export function buildFallbackHeadline(agentOutput: string): string {
  const cleaned = redactSensitive(stripMarkdown(agentOutput));
  if (!cleaned) return '';
  // Prefer the first sentence; fall back to the whole cleaned string.
  const firstSentence = cleaned.split(/(?<=[.!?])\s/)[0] ?? cleaned;
  const candidate = firstSentence.length >= 12 ? firstSentence : cleaned;
  return truncateAtWord(candidate.replace(/[.!?]+$/, ''), SHARE_HEADLINE_MAX);
}

/**
 * Normalises a headline that may have come from an LLM: strips wrapping quotes,
 * surrounding markdown, secrets/paths/emails, collapses whitespace, and caps
 * length. Returns an empty string when the result is unusable so the caller can
 * fall back.
 */
export function sanitizeHeadline(raw: string): string {
  const unquoted = raw.trim().replace(/^["'`]+|["'`]+$/g, '');
  const cleaned = redactSensitive(stripMarkdown(unquoted));
  if (cleaned.length < 3) return '';
  return truncateAtWord(cleaned.replace(/[.!?]+$/, ''), SHARE_HEADLINE_MAX);
}

/** Localized templates for {@link buildShareCaption}. See `share.default*` / `share.captionWithHeadline` in `en.ts`. */
export interface ShareCaptionTemplates {
  /** Shown when there's no usable headline. */
  emptyFallback: string;
  /** Template used when a headline is available; must contain a `{headline}` placeholder. */
  withHeadline: string;
}

/** English defaults, used when a caller doesn't pass locale-aware templates (e.g. tests). */
const DEFAULT_CAPTION_TEMPLATES: ShareCaptionTemplates = {
  emptyFallback: 'Look what my OpenHuman agent just did.',
  withHeadline: '{headline}. Made with my OpenHuman agent.',
};

/**
 * Builds the default social caption seeded from a headline. The card carries
 * the branding, so the caption stays short and leaves room for the link the
 * share-intent appends. The user can edit this before posting.
 *
 * `templates` should be sourced from `useT()` at the UI boundary so non-English
 * users don't get an English caption; it defaults to English for callers that
 * don't have a locale (tests, non-UI callers).
 */
export function buildShareCaption(
  headline: string,
  templates: ShareCaptionTemplates = DEFAULT_CAPTION_TEMPLATES
): string {
  const clean = headline.trim();
  const base = clean
    ? templates.withHeadline.replace('{headline}', clean)
    : templates.emptyFallback;
  const capped = truncateAtWord(base, TWEET_MAX - 30); // headroom for the link.
  if (capped !== base) {
    console.debug(`${LOG_PREFIX} caption truncated len=${base.length}`);
  }
  return capped;
}
