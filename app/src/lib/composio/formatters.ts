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
export function formatTriggerLabel(
  slug: string | null | undefined,
  opts?: { overrides?: Record<string, string> }
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

  return tokens
    .map(token => {
      if (token.toUpperCase() === 'GITHUB') return 'GitHub';
      return token.charAt(0).toUpperCase() + token.slice(1).toLowerCase();
    })
    .join(' ');
}
