/**
 * Display metadata for Composio toolkits shown in the Skills grid.
 *
 * The backend allowlist (`GET /agent-integrations/composio/toolkits`)
 * returns plain toolkit slugs. This table turns each slug into a
 * humanised name, description, category, and emoji icon so the
 * `UnifiedSkillCard` can render them next to regular skills without
 * special-casing.
 *
 * Unknown slugs fall back to a generic entry with a title-cased name —
 * new backend toolkits will still render, just without custom copy.
 */
import type { SkillCategory } from '../skills/SkillCategoryFilter';

export interface ComposioToolkitMeta {
  /** Toolkit slug as returned by the backend, e.g. `"gmail"`. */
  slug: string;
  /** Display name shown on the card, e.g. `"Gmail"`. */
  name: string;
  /** Short description shown on the card. */
  description: string;
  /** Which Skills page category to group the card under. */
  category: SkillCategory;
  /** Emoji fallback icon. Replace with SVGs later if desired. */
  icon: string;
}

const CATALOG: Record<string, Omit<ComposioToolkitMeta, 'slug'>> = {
  gmail: {
    name: 'Gmail',
    description: 'Read, search, and send email through your Google account.',
    category: 'Productivity',
    icon: '\u2709\uFE0F',
  },
  googlecalendar: {
    name: 'Google Calendar',
    description: 'List and manage events across your Google calendars.',
    category: 'Productivity',
    icon: '\uD83D\uDCC5',
  },
  googledrive: {
    name: 'Google Drive',
    description: 'Browse and fetch files from your Google Drive.',
    category: 'Productivity',
    icon: '\uD83D\uDCC2',
  },
  notion: {
    name: 'Notion',
    description: 'Read and edit pages, databases, and comments in Notion.',
    category: 'Productivity',
    icon: '\uD83D\uDCDD',
  },
  github: {
    name: 'GitHub',
    description: 'Inspect repos, issues, and pull requests on GitHub.',
    category: 'Tools & Automation',
    icon: '\uD83D\uDC0D',
  },
  slack: {
    name: 'Slack',
    description: 'Send messages and read channel history in Slack.',
    category: 'Social',
    icon: '\uD83D\uDCAC',
  },
  linear: {
    name: 'Linear',
    description: 'Triage and update issues in your Linear workspace.',
    category: 'Tools & Automation',
    icon: '\uD83D\uDCCB',
  },
};

export const KNOWN_COMPOSIO_TOOLKITS = Object.freeze(Object.keys(CATALOG));

export function composioToolkitMeta(slug: string): ComposioToolkitMeta {
  const key = slug.toLowerCase();
  const hit = CATALOG[key];
  if (hit) return { slug: key, ...hit };
  // Fallback: title-case the slug and bucket it under "Other".
  const name = key.charAt(0).toUpperCase() + key.slice(1);
  return {
    slug: key,
    name,
    description: `Composio integration for ${name}.`,
    category: 'Other',
    icon: '\uD83D\uDD0C',
  };
}
