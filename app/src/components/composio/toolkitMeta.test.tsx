import { describe, expect, it } from 'vitest';

import { composioToolkitMeta, KNOWN_COMPOSIO_TOOLKITS } from './toolkitMeta';

describe('composioToolkitMeta', () => {
  it('ships the full Composio managed-auth catalog fallback', () => {
    expect(KNOWN_COMPOSIO_TOOLKITS).toHaveLength(118);
    expect(KNOWN_COMPOSIO_TOOLKITS).toContain('gmail');
    expect(KNOWN_COMPOSIO_TOOLKITS).toContain('discord');
    expect(KNOWN_COMPOSIO_TOOLKITS).toContain('supabase');
    expect(KNOWN_COMPOSIO_TOOLKITS).toContain('zoom');
  });

  it('preserves canonical names for managed-auth toolkits and renders logo URLs', () => {
    const gmail = composioToolkitMeta('gmail');
    const calendar = composioToolkitMeta('google_calendar');

    expect(gmail.name).toBe('Gmail');
    expect(gmail.logoUrl).toContain('/gmail');
    expect(gmail.permissionLabel).toBe('Docs, files, tasks, and workspace data');

    expect(calendar.slug).toBe('googlecalendar');
    expect(calendar.name).toBe('Google Calendar');
    expect(calendar.logoUrl).toContain('/googlecalendar');
  });

  it('falls back cleanly for unknown slugs', () => {
    const meta = composioToolkitMeta('my_custom_toolkit');

    expect(meta.slug).toBe('my_custom_toolkit');
    expect(meta.name).toBe('My Custom Toolkit');
    expect(meta.logoUrl).toContain('/my_custom_toolkit');
  });
});
