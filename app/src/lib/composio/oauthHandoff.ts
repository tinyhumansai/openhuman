/**
 * OAuth handoff helpers for Meta-owned Composio toolkits (#1952).
 *
 * Instagram and Facebook share Meta's OAuth rate limits. The UI uses these
 * helpers to detect rate-limit failures and avoid duplicate authorize calls.
 */

/** Toolkits whose OAuth flows are hosted by Meta. */
export const META_OAUTH_TOOLKITS = ['instagram', 'facebook'] as const;

export type MetaOAuthToolkit = (typeof META_OAUTH_TOOLKITS)[number];

export function isMetaOAuthToolkit(slug: string): slug is MetaOAuthToolkit {
  const key = slug.trim().toLowerCase();
  return (META_OAUTH_TOOLKITS as readonly string[]).includes(key);
}

/** True when an error message looks like an OAuth / Meta rate limit (HTTP 429). */
export function isOAuthRateLimitedError(err: unknown): boolean {
  if (!err) return false;
  const msg =
    err instanceof Error
      ? err.message
      : typeof err === 'object' && err !== null && 'message' in err
        ? String((err as { message?: unknown }).message ?? '')
        : String(err);
  const lower = msg.toLowerCase();
  return (
    lower.includes('429') ||
    lower.includes('too many requests') ||
    lower.includes('rate limit') ||
    lower.includes('rate_limit') ||
    lower.includes('ratelimited')
  );
}

/** User-facing copy when Meta OAuth is rate-limited. */
export function metaOAuthRateLimitMessage(toolkitName: string): string {
  const normalizedName = toolkitName.trim().toLowerCase();
  const accountHint =
    normalizedName === 'instagram'
      ? ' Use an Instagram Business or Creator account — personal accounts are not supported.'
      : normalizedName === 'facebook'
        ? ' Confirm the Facebook account has access to the relevant Page or Business Manager.'
        : '';
  return (
    `Meta is temporarily rate-limiting ${toolkitName} sign-in (HTTP 429). ` +
    `Wait a few minutes before retrying and avoid clicking Connect repeatedly.${accountHint}`
  );
}
