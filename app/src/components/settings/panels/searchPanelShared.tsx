/*
 * Shared pieces of the web search settings: the provider swatch, role labels
 * and the small predicates every section needs to agree on (can this provider
 * run via TinyHumans right now, does connecting it need a key or a URL).
 *
 * Nothing here knows a provider by id except the cosmetic swatch tables; the
 * behavior is driven by the provider entry the core returned.
 */
import { createElement } from 'react';
import type { IconType } from 'react-icons';
import { SiBrave, SiGooglegemini, SiSearxng } from 'react-icons/si';

import { cn } from '../../../lib/cn';
import type {
  SearchProviderInfo,
  SearchProviderStatus,
  SearchRole,
} from '../../../utils/tauriCommands/config';
import type { BadgeVariant } from '../../ui/Badge';

export type Translate = (key: string) => string;

/** The roles the core exposes, in display order. */
export const SEARCH_ROLES: readonly SearchRole[] = ['search', 'answer', 'contents'];

export const withProvider = (text: string, provider: string) =>
  text.replace('{provider}', provider);

export function roleTitle(role: SearchRole, t: Translate): string {
  if (role === 'answer') return t('settings.search.roleAnswer');
  if (role === 'contents') return t('settings.search.roleContents');
  return t('settings.search.roleSearch');
}

export function roleDescription(role: SearchRole, t: Translate): string {
  if (role === 'answer') return t('settings.search.roleAnswerDesc');
  if (role === 'contents') return t('settings.search.roleContentsDesc');
  return t('settings.search.roleSearchDesc');
}

/** "Search · Answer · Contents" — what a provider can do, in role order. */
export const rolesSummary = (provider: SearchProviderInfo, t: Translate): string =>
  SEARCH_ROLES.filter(role => provider.roles.includes(role))
    .map(role => roleTitle(role, t))
    .join(' · ');

export const STATUS_VARIANT: Record<SearchProviderStatus, BadgeVariant> = {
  ready: 'success',
  needs_key: 'warning',
  sign_in_required: 'warning',
  disabled: 'neutral',
  search_off: 'neutral',
};

/** Badge text for a provider status. Literal keys so the i18n scanner sees them. */
export function statusLabel(status: SearchProviderStatus, t: Translate): string {
  switch (status) {
    case 'ready':
      return t('settings.search.statusReady');
    case 'needs_key':
      return t('settings.search.statusNeedsKey');
    case 'sign_in_required':
      return t('settings.search.statusSignInRequired');
    default:
      return t('settings.search.statusOff');
  }
}

/** True when the provider can run via TinyHumans in this session. */
export const canUseManaged = (provider: SearchProviderInfo, managedUnavailable: boolean) =>
  !managedUnavailable && provider.routes.includes('managed') && provider.managed_available;

/**
 * Enabled, but only over the managed route, with no account behind this
 * session.
 *
 * The core reports providers like Exa and Gemini as `enabled` with
 * `route: 'managed'` because they are on by default via TinyHumans. In a local
 * session that route is unreachable, so rendering them under "Connected" with
 * an on-toggle claimed a working provider the agent cannot actually call --
 * the row said "Connected", "via TinyHumans" and "Sign in required" all at
 * once. They belong with the providers you could connect with your own key.
 */
export const isStrandedManaged = (
  provider: SearchProviderInfo,
  managedUnavailable: boolean
): boolean =>
  managedUnavailable &&
  provider.enabled &&
  provider.route === 'managed' &&
  // Only move it if the catalogue can actually offer something: a provider
  // with no direct route would land in a blocked tile with no switch and no
  // action, leaving it enabled in core settings and uncontrollable from here.
  // Those keep their Connected row, where the toggle can still turn them off.
  provider.routes.includes('direct');

/** A provider that can be reached with the user's own key or instance. */
export const canUseDirect = (provider: SearchProviderInfo) => provider.routes.includes('direct');

/** SearXNG-style providers report an instance URL instead of (or besides) a key. */
export const hasBaseUrl = (provider: SearchProviderInfo) => provider.base_url !== undefined;

/**
 * Whether turning the provider on over its direct route first needs input:
 * a key it has not got (unless the key is optional), or an instance URL that
 * is empty.
 */
export const directNeedsSetup = (provider: SearchProviderInfo) =>
  (provider.takes_key && !provider.key_configured && !provider.key_optional) ||
  (hasBaseUrl(provider) && !provider.base_url);

const SWATCH_ICONS: Record<string, IconType> = {
  gemini: SiGooglegemini,
  brave: SiBrave,
  searxng: SiSearxng,
};

const SWATCH_TONES: Record<string, string> = {
  exa: 'bg-[#1F40ED]',
  gemini: 'bg-[#4285F4]',
  tinyfish: 'bg-[#0284C7]',
  parallel: 'bg-[#18181B]',
  brave: 'bg-[#FB542B]',
  tavily: 'bg-[#6D28D9]',
  querit: 'bg-[#0F766E]',
  seltz: 'bg-[#DB2777]',
  searxng: 'bg-[#3050FF]',
  keenable: 'bg-[#005CFF]',
};

/**
 * A search provider's mark on its tone swatch — the same 36px tile the LLM
 * provider list uses, so the two pages read as one design. Providers without
 * a bundled mark get their initial, which is unambiguous next to the name.
 */
export const SearchProviderSwatch = ({
  id,
  label,
  size = 'md',
}: {
  id: string;
  label: string;
  size?: 'sm' | 'md';
}) => {
  const icon = SWATCH_ICONS[id];
  return (
    <span
      aria-hidden
      data-slot="provider-swatch"
      className={cn(
        'flex shrink-0 items-center justify-center rounded-lg font-semibold text-content-inverted ring-1 ring-content-inverted/30',
        size === 'sm' ? 'h-7 w-7 text-[11px]' : 'h-9 w-9 text-xs',
        SWATCH_TONES[id] ?? 'bg-[#27272A]'
      )}>
      {icon
        ? createElement(icon, { className: size === 'sm' ? 'h-3.5 w-3.5' : 'h-4 w-4' })
        : label.trim().charAt(0).toUpperCase() || '?'}
    </span>
  );
};
