import type { ComposioConnection } from '../../lib/composio/types';

export function deriveConnectionLabel(c: ComposioConnection): string | null {
  for (const value of [c.accountEmail, c.workspace, c.username]) {
    const normalized = value?.trim();
    if (normalized) return normalized;
  }
  return null;
}

/**
 * The Composio error slug for missing required fields (code 612). Matching
 * on the slug string is more precise than matching the numeric code, which
 * could appear in unrelated messages (e.g. port numbers, resource IDs).
 */
const COMPOSIO_MISSING_REQUIRED_FIELDS_SLUG = 'ConnectedAccount_MissingRequiredFields';

/**
 * Validate an Atlassian subdomain. Accepts the short form used in
 * `<subdomain>.atlassian.net` — alphanumerics and hyphens, 1-63 chars,
 * no leading/trailing hyphens. Rejects full URLs so users are not confused
 * about what to paste.
 *
 * Retained for backwards compatibility with consumers that imported the
 * helper directly. The registry in `toolkitRequiredFields.ts` uses the
 * same regex via `validateSubdomainLabel`, shared with Dynamics 365.
 */
export function isValidAtlassianSubdomain(value: string): boolean {
  return /^[a-z0-9][a-z0-9-]{0,61}[a-z0-9]$|^[a-z0-9]$/i.test(value.trim());
}

/**
 * Detect a `ConnectedAccount_MissingRequiredFields` (code 612) error from
 * the backend/Composio. Returns true if the thrown error message contains
 * the known slug. Matching only on the slug avoids false positives from
 * unrelated messages that happen to contain the numeric code "612".
 * Safe to call with any value — returns false for null/non-Error.
 */
export function isMissingRequiredFieldsError(err: unknown): boolean {
  if (!err) return false;
  const msg = err instanceof Error ? err.message : String(err);
  return msg.includes(COMPOSIO_MISSING_REQUIRED_FIELDS_SLUG);
}

/**
 * Pull a human-readable message out of a JSON error payload embedded in a
 * message, e.g. `Backend returned 400 Bad Request: {"success":false,
 * "error":"No auth config found for toolkit \"reddit\""}`.
 *
 * Returns null when there is no JSON object, when it does not parse, or when
 * it parses but carries no readable string field — the caller then keeps its
 * previous behaviour. `error` / `message` / `detail` and a nested
 * `error.message` cover the backend envelope and the provider shapes.
 */
function extractEmbeddedErrorMessage(raw: string): string | null {
  const start = raw.indexOf('{');
  if (start === -1) return null;
  const end = raw.lastIndexOf('}');
  if (end <= start) return null;

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw.slice(start, end + 1));
  } catch {
    return null;
  }
  if (parsed === null || typeof parsed !== 'object') return null;

  const record = parsed as Record<string, unknown>;
  const nested = record.error;
  const candidates: unknown[] = [
    typeof nested === 'object' && nested !== null
      ? (nested as Record<string, unknown>).message
      : undefined,
    typeof nested === 'string' ? nested : undefined,
    record.message,
    record.detail,
  ];
  for (const candidate of candidates) {
    if (typeof candidate === 'string' && candidate.trim()) return candidate.trim();
  }
  return null;
}

/**
 * Return a safe, user-facing summary of an authorization failure. Strips the
 * raw backend URL and JSON payload from the message so sensitive Composio
 * internals are never shown in the UI.
 */
export function sanitizeAuthError(err: unknown): string {
  if (isMissingRequiredFieldsError(err)) {
    // Never surface raw 612 payloads — callers should handle this separately.
    return 'A required field is missing. Please provide the missing details and try again.';
  }
  if (!err) return 'Something went wrong.';
  const raw = err instanceof Error ? err.message : String(err);

  // A JSON payload the backend embedded carries the actionable text — the 400
  // that names the unconfigured toolkit (#5507). Read it out *before* the
  // guard below trims the message at the first `{` and discards it.
  const embedded = extractEmbeddedErrorMessage(raw);
  if (embedded) return embedded;

  // Strip any URL that looks like a backend endpoint so it is not displayed.
  const stripped = raw.replace(/https?:\/\/[^\s"]+/g, '<backend>');

  // Trim at the first occurrence of a JSON blob to avoid leaking payloads.
  // The URL stripping above may consume the `:` before `{`, so we match
  // the optional colon and any surrounding whitespace before the `{`.
  // This covers both `: {"error"...}` and the bare ` {"error"...}` form.
  const jsonIdx = stripped.search(/\s*:?\s*\{"error"/);
  // Fall back to trimming at any bare `{` that follows whitespace if we
  // did not find a `{"error"` form (defensive — handles other JSON shapes).
  const jsonIdxFallback = stripped.search(/\s\{/);
  const cutIdx =
    jsonIdx !== -1 ? jsonIdx : jsonIdxFallback !== -1 ? jsonIdxFallback : stripped.length;
  const trimmed = stripped.slice(0, cutIdx).trimEnd();

  // Collapse repeated colons / prefixes produced by the RPC error chain.
  // Apply iteratively until stable to handle nested wrapping.
  let result = trimmed;
  let prev: string;
  do {
    prev = result;
    result = result
      .replace(/^(Authorization failed:\s*)+/i, '')
      .replace(/^\[composio\]\s*authorize failed:\s*/i, '')
      .replace(/^Backend returned \d+[^:]*(?:for POST <backend>[^:]*)?:?\s*/i, '')
      .replace(/^Composio authorization failed:\s*/i, '')
      .trim();
  } while (result !== prev);

  // Nothing readable: if the message is still a raw JSON object, drop it as
  // the guard intended rather than showing the payload.
  if (result.startsWith('{')) return 'Authorization failed.';
  return result || 'Authorization failed.';
}
