/**
 * Display metadata for Composio toolkits shown in the Skills grid.
 *
 * The backend allowlist (`GET /agent-integrations/composio/toolkits`)
 * returns plain toolkit slugs. This table turns each slug into a
 * humanised name, description, category, and icon so the
 * `UnifiedSkillCard` can render them next to regular skills without
 * special-casing.
 *
 * Unknown slugs fall back to a generic entry with a title-cased name —
 * new backend toolkits will still render, just without custom copy.
 */
import type { ReactNode } from 'react';
import {
  SiFacebook,
  SiGithub,
  SiGmail,
  SiGooglecalendar,
  SiGoogledrive,
  SiGooglesheets,
  SiInstagram,
  SiLinear,
  SiNotion,
  SiReddit,
  SiSlack,
} from 'react-icons/si';

import type { SkillCategory } from '../skills/skillCategories';
import { SkillIconBadge } from '../skills/skillIcons';

export interface ComposioToolkitMeta {
  /** Toolkit slug as returned by the backend, e.g. `"gmail"`. */
  slug: string;
  /** Display name shown on the card, e.g. `"Gmail"`. */
  name: string;
  /** Short description shown on the card. */
  description: string;
  /** Which Skills page category to group the card under. */
  category: SkillCategory;
  /** Small branded icon rendered on the card and connect modal. */
  icon: ReactNode;
}

function GmailIcon() {
  return (
    <SkillIconBadge
      icon={SiGmail}
      label="Gmail"
      bgClassName="bg-white"
      iconClassName="text-[#EA4335]"
    />
  );
}

function GoogleCalendarIcon() {
  return (
    <SkillIconBadge
      icon={SiGooglecalendar}
      label="Google Calendar"
      bgClassName="bg-[#E8F0FE]"
      iconClassName="text-[#4285F4]"
    />
  );
}

function GoogleDriveIcon() {
  return (
    <SkillIconBadge
      icon={SiGoogledrive}
      label="Google Drive"
      bgClassName="bg-white"
      iconClassName="text-[#0F9D58]"
    />
  );
}

function NotionIcon() {
  return (
    <SkillIconBadge
      icon={SiNotion}
      label="Notion"
      bgClassName="bg-white"
      iconClassName="text-[#111111]"
    />
  );
}

function GitHubIcon() {
  return (
    <SkillIconBadge
      icon={SiGithub}
      label="GitHub"
      bgClassName="bg-[#111827]"
      iconClassName="text-white"
    />
  );
}

function SlackIcon() {
  return (
    <SkillIconBadge
      icon={SiSlack}
      label="Slack"
      bgClassName="bg-white"
      iconClassName="text-[#4A154B]"
    />
  );
}

function LinearIcon() {
  return (
    <SkillIconBadge
      icon={SiLinear}
      label="Linear"
      bgClassName="bg-[#0F172A]"
      iconClassName="text-white"
    />
  );
}

function FacebookIcon() {
  return (
    <SkillIconBadge
      icon={SiFacebook}
      label="Facebook"
      bgClassName="bg-[#1877F2]"
      iconClassName="text-white"
    />
  );
}

function GoogleSheetsIcon() {
  return (
    <SkillIconBadge
      icon={SiGooglesheets}
      label="Google Sheets"
      bgClassName="bg-[#E6F4EA]"
      iconClassName="text-[#0F9D58]"
    />
  );
}

function InstagramIcon() {
  return (
    <SkillIconBadge
      icon={SiInstagram}
      label="Instagram"
      bgClassName="bg-[radial-gradient(circle_at_30%_107%,_#fdf497_0%,_#fdf497_5%,_#fd5949_45%,_#d6249f_60%,_#285AEB_90%)]"
      iconClassName="text-white"
    />
  );
}

function RedditIcon() {
  return (
    <SkillIconBadge
      icon={SiReddit}
      label="Reddit"
      bgClassName="bg-[#FF4500]"
      iconClassName="text-white"
    />
  );
}

function GenericIntegrationIcon() {
  return (
    <span className="flex h-8 w-8 items-center justify-center rounded-xl bg-stone-100 text-stone-600 shadow-sm ring-1 ring-black/5">
      <svg className="h-[18px] w-[18px]" viewBox="0 0 24 24" aria-hidden="true" fill="none">
        <path
          d="M8 8h8v8H8zM5 12h3m8 0h3M12 5v3m0 8v3"
          stroke="currentColor"
          strokeWidth="1.7"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
    </span>
  );
}

const CATALOG: Record<string, Omit<ComposioToolkitMeta, 'slug'>> = {
  gmail: {
    name: 'Gmail',
    description: 'Read, search, and send email through your Google account.',
    category: 'Productivity',
    icon: <GmailIcon />,
  },
  googlecalendar: {
    name: 'Google Calendar',
    description: 'List and manage events across your Google calendars.',
    category: 'Productivity',
    icon: <GoogleCalendarIcon />,
  },
  google_calendar: {
    name: 'Google Calendar',
    description: 'List and manage events across your Google calendars.',
    category: 'Productivity',
    icon: <GoogleCalendarIcon />,
  },
  googledrive: {
    name: 'Google Drive',
    description: 'Browse and fetch files from your Google Drive.',
    category: 'Productivity',
    icon: <GoogleDriveIcon />,
  },
  google_drive: {
    name: 'Google Drive',
    description: 'Browse and fetch files from your Google Drive.',
    category: 'Productivity',
    icon: <GoogleDriveIcon />,
  },
  notion: {
    name: 'Notion',
    description: 'Read and edit pages, databases, and comments in Notion.',
    category: 'Productivity',
    icon: <NotionIcon />,
  },
  github: {
    name: 'GitHub',
    description: 'Inspect repos, issues, and pull requests on GitHub.',
    category: 'Tools & Automation',
    icon: <GitHubIcon />,
  },
  slack: {
    name: 'Slack',
    description: 'Send messages and read channel history in Slack.',
    category: 'Social',
    icon: <SlackIcon />,
  },
  linear: {
    name: 'Linear',
    description: 'Triage and update issues in your Linear workspace.',
    category: 'Tools & Automation',
    icon: <LinearIcon />,
  },
  facebook: {
    name: 'Facebook',
    description: 'Create posts, manage pages, and work with Facebook social data.',
    category: 'Social',
    icon: <FacebookIcon />,
  },
  google_sheets: {
    name: 'Google Sheets',
    description: 'Read, update, and organize spreadsheets in Google Sheets.',
    category: 'Productivity',
    icon: <GoogleSheetsIcon />,
  },
  googlesheets: {
    name: 'Google Sheets',
    description: 'Read, update, and organize spreadsheets in Google Sheets.',
    category: 'Productivity',
    icon: <GoogleSheetsIcon />,
  },
  instagram: {
    name: 'Instagram',
    description: 'Manage Instagram publishing, messaging, and social content workflows.',
    category: 'Social',
    icon: <InstagramIcon />,
  },
  reddit: {
    name: 'Reddit',
    description: 'Read posts, monitor communities, and participate in Reddit discussions.',
    category: 'Social',
    icon: <RedditIcon />,
  },
};

/**
 * Canonical toolkit slugs used as the default catalog when the backend
 * allowlist hasn't loaded yet. One entry per integration — CATALOG
 * handles alternate slug variants (e.g. `google_calendar` →
 * `googlecalendar`) so they don't need to appear here.
 */
export const KNOWN_COMPOSIO_TOOLKITS = Object.freeze([
  'gmail',
  'googlecalendar',
  'googledrive',
  'google_sheets',
  'notion',
  'github',
  'slack',
  'linear',
  'facebook',
  'instagram',
  'reddit',
]);

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
    icon: <GenericIntegrationIcon />,
  };
}
