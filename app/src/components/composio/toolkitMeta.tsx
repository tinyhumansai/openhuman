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
  /** Small branded icon rendered on the card and connect modal. */
  icon: ReactNode;
}

function BrandIcon({ bgClassName, children }: { bgClassName: string; children: ReactNode }) {
  return (
    <span
      className={`flex h-8 w-8 items-center justify-center rounded-xl shadow-sm ring-1 ring-black/5 ${bgClassName}`}>
      {children}
    </span>
  );
}

function GmailIcon() {
  return (
    <BrandIcon bgClassName="bg-white">
      <svg className="h-[18px] w-[18px]" viewBox="0 0 24 24" aria-hidden="true">
        <path
          d="M3 7.25 12 14l9-6.75V17a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7.25Z"
          fill="#EA4335"
          opacity="0.15"
        />
        <path d="M3 7.5 12 14l9-6.5" fill="none" stroke="#EA4335" strokeWidth="2" />
        <path d="M3 8v9a2 2 0 0 0 2 2h3V11.5L3 8Z" fill="#34A853" />
        <path d="M21 8v9a2 2 0 0 1-2 2h-3v-7.5L21 8Z" fill="#4285F4" />
        <path d="M8 19v-7.5l4 3 4-3V19H8Z" fill="#FBBC05" />
      </svg>
    </BrandIcon>
  );
}

function GoogleCalendarIcon() {
  return (
    <BrandIcon bgClassName="bg-[#E8F0FE]">
      <svg className="h-[18px] w-[18px]" viewBox="0 0 24 24" aria-hidden="true">
        <rect x="4" y="5" width="16" height="15" rx="3" fill="#4285F4" />
        <rect x="4" y="8" width="16" height="12" rx="0" fill="white" />
        <path d="M8 4v4M16 4v4" stroke="#4285F4" strokeWidth="2" strokeLinecap="round" />
        <text x="12" y="17" textAnchor="middle" fontSize="7" fontWeight="700" fill="#4285F4">
          31
        </text>
      </svg>
    </BrandIcon>
  );
}

function GoogleDriveIcon() {
  return (
    <BrandIcon bgClassName="bg-white">
      <svg className="h-[18px] w-[18px]" viewBox="0 0 24 24" aria-hidden="true">
        <path d="M7 4h5l5 8h-5L7 4Z" fill="#0F9D58" />
        <path d="M7 4 2 12l2.5 4h5L14 8 7 4Z" fill="#F4B400" />
        <path d="M9.5 16h10L22 12h-10L9.5 16Z" fill="#4285F4" />
      </svg>
    </BrandIcon>
  );
}

function NotionIcon() {
  return (
    <BrandIcon bgClassName="bg-white">
      <svg className="h-[18px] w-[18px]" viewBox="0 0 24 24" aria-hidden="true">
        <rect x="5" y="5" width="14" height="14" rx="1.8" fill="white" stroke="#111" />
        <path d="M9 16V8l6 8V8" fill="none" stroke="#111" strokeWidth="1.8" strokeLinecap="round" />
      </svg>
    </BrandIcon>
  );
}

function GitHubIcon() {
  return (
    <BrandIcon bgClassName="bg-[#111827]">
      <svg className="h-[18px] w-[18px]" viewBox="0 0 24 24" aria-hidden="true" fill="white">
        <path d="M12 3.5a8.5 8.5 0 0 0-2.69 16.56c.43.08.58-.19.58-.42v-1.48c-2.36.51-2.86-1-2.86-1-.39-.98-.95-1.25-.95-1.25-.77-.53.06-.52.06-.52.85.06 1.3.87 1.3.87.75 1.29 1.98.92 2.46.7.08-.55.3-.92.55-1.13-1.88-.21-3.86-.94-3.86-4.18 0-.92.33-1.67.87-2.25-.08-.21-.38-1.07.09-2.23 0 0 .71-.23 2.34.86a8.02 8.02 0 0 1 4.26 0c1.63-1.09 2.34-.86 2.34-.86.47 1.16.17 2.02.09 2.23.54.58.87 1.33.87 2.25 0 3.25-1.99 3.97-3.89 4.18.31.27.58.79.58 1.59v2.35c0 .23.15.51.59.42A8.5 8.5 0 0 0 12 3.5Z" />
      </svg>
    </BrandIcon>
  );
}

function SlackIcon() {
  return (
    <BrandIcon bgClassName="bg-white">
      <svg className="h-[18px] w-[18px]" viewBox="0 0 24 24" aria-hidden="true">
        <path
          d="M9.2 3.5a2.2 2.2 0 1 1 0 4.4H7v2.2a2.2 2.2 0 1 1-4.4 0A2.2 2.2 0 0 1 4.8 7.9H7V5.7a2.2 2.2 0 0 1 2.2-2.2Z"
          fill="#36C5F0"
        />
        <path
          d="M20.5 9.2a2.2 2.2 0 1 1-4.4 0V7h-2.2a2.2 2.2 0 1 1 0-4.4 2.2 2.2 0 0 1 2.2 2.2V7h2.2a2.2 2.2 0 0 1 2.2 2.2Z"
          fill="#2EB67D"
        />
        <path
          d="M14.8 20.5a2.2 2.2 0 1 1 0-4.4H17v-2.2a2.2 2.2 0 1 1 4.4 0 2.2 2.2 0 0 1-2.2 2.2H17v2.2a2.2 2.2 0 0 1-2.2 2.2Z"
          fill="#ECB22E"
        />
        <path
          d="M3.5 14.8a2.2 2.2 0 1 1 4.4 0V17h2.2a2.2 2.2 0 1 1 0 4.4 2.2 2.2 0 0 1-2.2-2.2V17H5.7a2.2 2.2 0 0 1-2.2-2.2Z"
          fill="#E01E5A"
        />
      </svg>
    </BrandIcon>
  );
}

function LinearIcon() {
  return (
    <BrandIcon bgClassName="bg-[#0F172A]">
      <svg className="h-[18px] w-[18px]" viewBox="0 0 24 24" aria-hidden="true" fill="white">
        <path d="M6 6h8.5v2H8v2.5h5.5v2H8V15h6.5v2H6V6Zm10.8-.25a1.45 1.45 0 1 1 0 2.9 1.45 1.45 0 0 1 0-2.9Zm-1.3 4.5h2.6V17h-2.6v-6.75Z" />
      </svg>
    </BrandIcon>
  );
}

function FacebookIcon() {
  return (
    <BrandIcon bgClassName="bg-[#1877F2]">
      <svg className="h-[18px] w-[18px]" viewBox="0 0 24 24" aria-hidden="true" fill="white">
        <path d="M13.4 20v-6.3h2.12l.32-2.48H13.4V9.61c0-.72.2-1.2 1.23-1.2H16V6.2c-.67-.07-1.34-.1-2.01-.1-1.99 0-3.35 1.21-3.35 3.43v1.66H8.4v2.48h2.24V20h2.76Z" />
      </svg>
    </BrandIcon>
  );
}

function GoogleSheetsIcon() {
  return (
    <BrandIcon bgClassName="bg-[#E6F4EA]">
      <svg className="h-[18px] w-[18px]" viewBox="0 0 24 24" aria-hidden="true">
        <path d="M8 3h7l5 5v11a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2Z" fill="#0F9D58" />
        <path d="M15 3v5h5" fill="#34A853" />
        <path
          d="M9 11h6M9 14h6M9 17h6M13 9v10"
          stroke="white"
          strokeWidth="1.4"
          strokeLinecap="round"
          opacity="0.95"
        />
      </svg>
    </BrandIcon>
  );
}

function InstagramIcon() {
  return (
    <BrandIcon bgClassName="bg-[radial-gradient(circle_at_30%_107%,_#fdf497_0%,_#fdf497_5%,_#fd5949_45%,_#d6249f_60%,_#285AEB_90%)]">
      <svg className="h-[18px] w-[18px]" viewBox="0 0 24 24" aria-hidden="true" fill="none">
        <rect x="6" y="6" width="12" height="12" rx="4" stroke="white" strokeWidth="1.8" />
        <circle cx="12" cy="12" r="3" stroke="white" strokeWidth="1.8" />
        <circle cx="16.3" cy="7.8" r="1" fill="white" />
      </svg>
    </BrandIcon>
  );
}

function RedditIcon() {
  return (
    <BrandIcon bgClassName="bg-[#FF4500]">
      <svg className="h-[18px] w-[18px]" viewBox="0 0 24 24" aria-hidden="true" fill="none">
        <circle cx="12" cy="13" r="5.5" fill="white" />
        <circle cx="9.3" cy="12.5" r="1" fill="#FF4500" />
        <circle cx="14.7" cy="12.5" r="1" fill="#FF4500" />
        <path
          d="M9.5 15.1c.7.6 1.52.9 2.5.9.98 0 1.8-.3 2.5-.9"
          stroke="#FF4500"
          strokeWidth="1.2"
          strokeLinecap="round"
        />
        <path d="m13 7.2 1.2-2.2 2.5.6" stroke="white" strokeWidth="1.3" strokeLinecap="round" />
        <circle cx="17.5" cy="5.7" r="1.2" fill="white" />
        <path
          d="M6.9 10.6c-.86 0-1.55-.72-1.55-1.6 0-.89.69-1.6 1.55-1.6.87 0 1.56.71 1.56 1.6 0 .88-.69 1.6-1.56 1.6Zm10.2 0c-.87 0-1.56-.72-1.56-1.6 0-.89.69-1.6 1.56-1.6.86 0 1.55.71 1.55 1.6 0 .88-.69 1.6-1.55 1.6Z"
          fill="white"
        />
      </svg>
    </BrandIcon>
  );
}

function GenericIntegrationIcon() {
  return (
    <BrandIcon bgClassName="bg-stone-100">
      <svg
        className="h-[18px] w-[18px] text-stone-600"
        viewBox="0 0 24 24"
        aria-hidden="true"
        fill="none">
        <path
          d="M8 8h8v8H8zM5 12h3m8 0h3M12 5v3m0 8v3"
          stroke="currentColor"
          strokeWidth="1.7"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
    </BrandIcon>
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
