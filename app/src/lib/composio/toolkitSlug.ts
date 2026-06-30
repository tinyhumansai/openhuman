const TOOLKIT_ALIASES: Record<string, string> = {
  feishu: 'larksuite',
  google_calendar: 'googlecalendar',
  google_drive: 'googledrive',
  google_sheets: 'googlesheets',
  lark: 'larksuite',
};

export function canonicalizeComposioToolkitSlug(slug: string): string {
  // `.trim()` keeps this in sync with the Rust `canonicalize_toolkit_slug`
  // (src/openhuman/composio/tools.rs) so a stray-whitespace slug can't diverge.
  const key = slug.trim().toLowerCase();
  return TOOLKIT_ALIASES[key] ?? key;
}
