/**
 * Display metadata for Composio toolkits shown in the Skills grid.
 *
 * We intentionally keep a local catalog of every Composio managed-auth
 * toolkit so the desktop UI can render a broad connection surface even
 * before the live backend allowlist expands further. The live toolkit
 * list still wins for runtime availability; this file provides stable
 * names, categories, descriptions, and logos for rendering.
 *
 * Source of truth for the managed-auth list:
 * https://docs.composio.dev/toolkits/managed-auth (118 toolkits as of
 * May 1, 2026).
 */
import { type ReactNode, useState } from 'react';

import { canonicalizeComposioToolkitSlug } from '../../lib/composio/toolkitSlug';
import type { SkillCategory } from '../skills/skillCategories';

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
  /** Composio-hosted logo URL for richer provider branding. */
  logoUrl: string;
  /** Short UX hint for what the user is authorizing. */
  permissionLabel: string;
}

interface ManagedToolkitEntry {
  slug: string;
  name: string;
}

const MANAGED_COMPOSIO_TOOLKITS: readonly ManagedToolkitEntry[] = Object.freeze([
  { slug: 'airtable', name: 'Airtable' },
  { slug: 'apaleo', name: 'Apaleo' },
  { slug: 'asana', name: 'Asana' },
  { slug: 'attio', name: 'Attio' },
  { slug: 'basecamp', name: 'Basecamp' },
  { slug: 'bitbucket', name: 'Bitbucket' },
  { slug: 'blackbaud', name: 'Blackbaud' },
  { slug: 'boldsign', name: 'Boldsign' },
  { slug: 'box', name: 'Box' },
  { slug: 'cal', name: 'Cal' },
  { slug: 'calendly', name: 'Calendly' },
  { slug: 'canva', name: 'Canva' },
  { slug: 'capsule_crm', name: 'Capsule CRM' },
  { slug: 'clickup', name: 'ClickUp' },
  { slug: 'confluence', name: 'Confluence' },
  { slug: 'contentful', name: 'Contentful' },
  { slug: 'convex', name: 'Convex' },
  { slug: 'crowdin', name: 'Crowdin' },
  { slug: 'dart', name: 'Dart' },
  { slug: 'dialpad', name: 'Dialpad' },
  { slug: 'digital_ocean', name: 'DigitalOcean' },
  { slug: 'discord', name: 'Discord' },
  { slug: 'discordbot', name: 'Discord Bot' },
  { slug: 'dropbox', name: 'Dropbox' },
  { slug: 'dub', name: 'Dub' },
  { slug: 'dynamics365', name: 'Dynamics 365' },
  { slug: 'eventbrite', name: 'Eventbrite' },
  { slug: 'excel', name: 'Excel' },
  { slug: 'exist', name: 'Exist' },
  { slug: 'facebook', name: 'Facebook' },
  { slug: 'fathom', name: 'Fathom' },
  { slug: 'figma', name: 'Figma' },
  { slug: 'freeagent', name: 'Freeagent' },
  { slug: 'freshbooks', name: 'FreshBooks' },
  { slug: 'github', name: 'GitHub' },
  { slug: 'gitlab', name: 'GitLab' },
  { slug: 'gmail', name: 'Gmail' },
  { slug: 'googleads', name: 'Google Ads' },
  { slug: 'google_analytics', name: 'Google Analytics' },
  { slug: 'googlebigquery', name: 'Google BigQuery' },
  { slug: 'googlecalendar', name: 'Google Calendar' },
  { slug: 'google_classroom', name: 'Google Classroom' },
  { slug: 'googledocs', name: 'Google Docs' },
  { slug: 'googledrive', name: 'Google Drive' },
  { slug: 'google_maps', name: 'Google Maps' },
  { slug: 'googlemeet', name: 'Google Meet' },
  { slug: 'googlephotos', name: 'Google Photos' },
  { slug: 'google_search_console', name: 'Google Search Console' },
  { slug: 'googlesheets', name: 'Google Sheets' },
  { slug: 'googleslides', name: 'Google Slides' },
  { slug: 'googlesuper', name: 'Google Super' },
  { slug: 'googletasks', name: 'Google Tasks' },
  { slug: 'gorgias', name: 'Gorgias' },
  { slug: 'gumroad', name: 'Gumroad' },
  { slug: 'harvest', name: 'Harvest' },
  { slug: 'hubspot', name: 'HubSpot' },
  { slug: 'hugging_face', name: 'Hugging Face' },
  { slug: 'instagram', name: 'Instagram' },
  { slug: 'intercom', name: 'Intercom' },
  { slug: 'jira', name: 'Jira' },
  { slug: 'kit', name: 'Kit' },
  { slug: 'linear', name: 'Linear' },
  { slug: 'linkedin', name: 'LinkedIn' },
  { slug: 'linkhut', name: 'Linkhut' },
  { slug: 'mailchimp', name: 'Mailchimp' },
  { slug: 'microsoft_teams', name: 'Microsoft Teams' },
  { slug: 'miro', name: 'Miro' },
  { slug: 'monday', name: 'Monday' },
  { slug: 'moneybird', name: 'Moneybird' },
  { slug: 'mural', name: 'Mural' },
  { slug: 'notion', name: 'Notion' },
  { slug: 'omnisend', name: 'Omnisend' },
  { slug: 'one_drive', name: 'OneDrive' },
  { slug: 'outlook', name: 'Outlook' },
  { slug: 'pagerduty', name: 'PagerDuty' },
  { slug: 'prisma', name: 'Prisma' },
  { slug: 'productboard', name: 'Productboard' },
  { slug: 'pushbullet', name: 'Pushbullet' },
  { slug: 'quickbooks', name: 'QuickBooks' },
  { slug: 'reddit', name: 'Reddit' },
  { slug: 'reddit_ads', name: 'Reddit Ads' },
  { slug: 'roam', name: 'Roam' },
  { slug: 'salesforce', name: 'Salesforce' },
  { slug: 'sentry', name: 'Sentry' },
  { slug: 'servicem8', name: 'Servicem8' },
  { slug: 'share_point', name: 'SharePoint' },
  { slug: 'shippo', name: 'Shippo' },
  { slug: 'slack', name: 'Slack' },
  { slug: 'slackbot', name: 'Slackbot' },
  { slug: 'splitwise', name: 'Splitwise' },
  { slug: 'square', name: 'Square' },
  { slug: 'stack_exchange', name: 'Stack Exchange' },
  { slug: 'strava', name: 'Strava' },
  { slug: 'stripe', name: 'Stripe' },
  { slug: 'supabase', name: 'Supabase' },
  { slug: 'ticketmaster', name: 'Ticketmaster' },
  { slug: 'ticktick', name: 'Ticktick' },
  { slug: 'timely', name: 'Timely' },
  { slug: 'todoist', name: 'Todoist' },
  { slug: 'toneden', name: 'Toneden' },
  { slug: 'trello', name: 'Trello' },
  { slug: 'typeform', name: 'Typeform' },
  { slug: 'wakatime', name: 'WakaTime' },
  { slug: 'webex', name: 'Webex' },
  { slug: 'whatsapp', name: 'WhatsApp' },
  { slug: 'wrike', name: 'Wrike' },
  { slug: 'yandex', name: 'Yandex' },
  { slug: 'ynab', name: 'YNAB' },
  { slug: 'youtube', name: 'YouTube' },
  { slug: 'zendesk', name: 'Zendesk' },
  { slug: 'zoho', name: 'Zoho' },
  { slug: 'zoho_bigin', name: 'Zoho Bigin' },
  { slug: 'zoho_books', name: 'Zoho Books' },
  { slug: 'zoho_desk', name: 'Zoho Desk' },
  { slug: 'zoho_inventory', name: 'Zoho Inventory' },
  { slug: 'zoho_invoice', name: 'Zoho Invoice' },
  { slug: 'zoho_mail', name: 'Zoho Mail' },
  { slug: 'zoom', name: 'Zoom' },
]);

const MANAGED_TOOLKIT_NAME_BY_SLUG = new Map(
  MANAGED_COMPOSIO_TOOLKITS.map(entry => [entry.slug, entry.name])
);

const CHAT_KEYWORDS = ['discord', 'slack', 'teams', 'webex', 'whatsapp', 'dialpad'];
const SOCIAL_KEYWORDS = [
  'facebook',
  'instagram',
  'linkedin',
  'reddit',
  'youtube',
  'stack_exchange',
];
const PRODUCTIVITY_KEYWORDS = [
  'gmail',
  'calendar',
  'drive',
  'docs',
  'doc',
  'sheets',
  'slides',
  'tasks',
  'todoist',
  'trello',
  'notion',
  'box',
  'dropbox',
  'sharepoint',
  'one_drive',
  'onedrive',
  'outlook',
  'miro',
  'mural',
  'monday',
  'clickup',
  'linear',
  'jira',
  'confluence',
  'asana',
  'basecamp',
  'wrike',
  'cal',
  'calendly',
  'typeform',
  'excel',
  'figma',
  'google',
];
const PLATFORM_KEYWORDS = [
  'github',
  'gitlab',
  'bitbucket',
  'digital_ocean',
  'contentful',
  'supabase',
  'convex',
  'prisma',
  'sentry',
  'stripe',
  'salesforce',
  'hubspot',
  'quickbooks',
  'zendesk',
  'zoho',
];

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

function ComposioLogoBadge({ slug, name }: { slug: string; name: string }) {
  const [failed, setFailed] = useState(false);
  const logoUrl = composioLogoUrl(slug);

  if (failed) {
    return <GenericIntegrationIcon />;
  }

  return (
    <span className="flex h-8 w-8 items-center justify-center overflow-hidden rounded-xl bg-white shadow-sm ring-1 ring-black/5">
      <img
        src={logoUrl}
        alt={`${name} logo`}
        className="h-full w-full object-contain p-1"
        loading="lazy"
        onError={() => setFailed(true)}
      />
    </span>
  );
}

function composioLogoUrl(slug: string): string {
  return `https://logos.composio.dev/api/${slug}`;
}

function guessCategory(slug: string, name: string): SkillCategory {
  const key = `${slug} ${name}`.toLowerCase();
  if (CHAT_KEYWORDS.some(keyword => key.includes(keyword))) return 'Chat';
  if (SOCIAL_KEYWORDS.some(keyword => key.includes(keyword))) return 'Social';
  if (PRODUCTIVITY_KEYWORDS.some(keyword => key.includes(keyword))) return 'Productivity';
  if (PLATFORM_KEYWORDS.some(keyword => key.includes(keyword))) return 'Platform';
  return 'Tools & Automation';
}

function defaultDescription(name: string, category: SkillCategory): string {
  switch (category) {
    case 'Chat':
      return `Connect ${name} for messaging, inbox, and team communication workflows.`;
    case 'Social':
      return `Connect ${name} for social publishing, community, and audience workflows.`;
    case 'Productivity':
      return `Connect ${name} for documents, planning, file, and day-to-day productivity workflows.`;
    case 'Platform':
      return `Connect ${name} for developer, platform, CRM, and business system workflows.`;
    default:
      return `Connect ${name} through Composio managed auth.`;
  }
}

function permissionLabelFor(category: SkillCategory): string {
  switch (category) {
    case 'Chat':
      return 'Messages, channels, and communication data';
    case 'Social':
      return 'Posts, profiles, and social content';
    case 'Productivity':
      return 'Docs, files, tasks, and workspace data';
    case 'Platform':
      return 'Repos, records, tickets, and system data';
    default:
      return 'Connected account data';
  }
}

function prettifyUnknownSlug(slug: string): string {
  return slug
    .split(/[_-]+/)
    .filter(Boolean)
    .map(part => part.charAt(0).toUpperCase() + part.slice(1))
    .join(' ');
}

/**
 * Canonical toolkit slugs used as the default catalog when the backend
 * allowlist hasn't loaded yet. One entry per Composio managed-auth
 * integration.
 */
export const KNOWN_COMPOSIO_TOOLKITS = Object.freeze(
  MANAGED_COMPOSIO_TOOLKITS.map(entry => entry.slug)
);

export function composioToolkitMeta(slug: string): ComposioToolkitMeta {
  const key = canonicalizeComposioToolkitSlug(slug);
  const name = MANAGED_TOOLKIT_NAME_BY_SLUG.get(key) ?? prettifyUnknownSlug(key);
  const category = guessCategory(key, name);
  return {
    slug: key,
    name,
    description: defaultDescription(name, category),
    category,
    icon: <ComposioLogoBadge slug={key} name={name} />,
    logoUrl: composioLogoUrl(key),
    permissionLabel: permissionLabelFor(category),
  };
}
