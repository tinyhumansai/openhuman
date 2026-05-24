/**
 * Formats a Composio trigger slug into a human-readable label.
 *
 * Example: GOOGLECALENDAR_GOOGLE_CALENDAR_EVENT_CREATED_TRIGGER
 * -> Google Calendar Event Created
 *
 * Rules:
 * 1. empty/null input -> return ''
 * 2. opts.overrides[slug] wins if present
 * 3. strip trailing _TRIGGER (case-insensitive)
 * 4. dedupe leading provider prefix when it reappears
 * 5. split on _, title-case each token, join with space
 */
/**
 * Parse a classified Composio error (`[composio:error:<class>] …`) for UI copy.
 */
export function formatComposioToolError(raw: string | null | undefined): string {
  if (!raw) return '';
  const match = /^\[composio:error:([a-z_]+)\]\s*(.*)$/is.exec(raw.trim());
  if (!match) return raw.trim();

  const [, className, body] = match;
  switch (className) {
    case 'validation':
      return body || 'Invalid tool arguments.';
    case 'insufficient_scope':
      return body || 'Reconnect this integration and grant the requested permissions.';
    case 'rate_limited':
      return body || 'The upstream service is rate-limiting requests. Try again shortly.';
    case 'gateway':
      return body || 'Temporary connection issue. Try again in a moment.';
    default:
      return body || raw.trim();
  }
}

export function formatTriggerLabel(
  slug: string | null | undefined,
  opts?: { overrides?: Record<string, string>; toolkit?: string | null }
): string {
  if (!slug) return '';
  if (opts?.overrides && Object.prototype.hasOwnProperty.call(opts.overrides, slug)) {
    return opts.overrides[slug] ?? '';
  }

  // Strip trailing _TRIGGER (case-insensitive)
  const workingSlug = slug.replace(/_TRIGGER$/i, '');

  const tokens = workingSlug.split('_').filter(t => t.length > 0);

  // Dedupe leading provider prefix
  // e.g. GOOGLECALENDAR_GOOGLE_CALENDAR_EVENT_CREATED -> drop GOOGLECALENDAR
  if (tokens.length > 1) {
    const first = tokens[0].toUpperCase();
    const second = tokens[1].toUpperCase();

    if (first === second) {
      tokens.shift();
    } else if (tokens.length > 2) {
      const third = tokens[2].toUpperCase();
      if (first === second + third) {
        tokens.shift();
      }
    }
  }

  // Strip remaining leading toolkit tokens when the caller supplies the
  // toolkit slug/name — keeps the label focused on the *event* part.
  // e.g. with toolkit='googlecalendar' or 'Google Calendar':
  //   GOOGLE CALENDAR EVENT CREATED -> EVENT CREATED
  if (opts?.toolkit && tokens.length > 1) {
    const toolkitTokens = opts.toolkit
      .toUpperCase()
      .split(/[\s_]+/)
      .filter(t => t.length > 0);
    // Build a virtual concatenation of the toolkit ("GOOGLECALENDAR") so we
    // can also drop a single-glued token like 'GOOGLECALENDAR' that maps to
    // multiple display words.
    const toolkitGlued = toolkitTokens.join('');

    // Drop a single-token gluing first.
    if (tokens[0].toUpperCase() === toolkitGlued && tokens.length > 1) {
      tokens.shift();
    }

    // Then drop consecutive matching tokens, stopping when something else
    // appears (so we don't accidentally swallow a real event word).
    let i = 0;
    while (
      i < toolkitTokens.length &&
      tokens.length > 1 &&
      tokens[0].toUpperCase() === toolkitTokens[i]
    ) {
      tokens.shift();
      i += 1;
    }
  }

  return tokens
    .map(token => {
      if (token.toUpperCase() === 'GITHUB') return 'GitHub';
      return token.charAt(0).toUpperCase() + token.slice(1).toLowerCase();
    })
    .join(' ');
}
