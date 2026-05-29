const TOOLKIT_ALIASES: Record<string, string> = {
  feishu: 'larksuite',
  google_calendar: 'googlecalendar',
  google_drive: 'googledrive',
  google_sheets: 'googlesheets',
  lark: 'larksuite',
};

export function canonicalizeComposioToolkitSlug(slug: string): string {
  const key = slug.toLowerCase();
  return TOOLKIT_ALIASES[key] ?? key;
}
