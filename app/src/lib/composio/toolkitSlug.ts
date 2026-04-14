const TOOLKIT_ALIASES: Record<string, string> = {
  google_calendar: 'googlecalendar',
  google_drive: 'googledrive',
  google_sheets: 'googlesheets',
};

export function canonicalizeComposioToolkitSlug(slug: string): string {
  const key = slug.toLowerCase();
  return TOOLKIT_ALIASES[key] ?? key;
}
