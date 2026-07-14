/**
 * Shared utilities for the Meetings composer.
 *
 * Centralises the platform metadata, display-name resolution helpers that were
 * previously embedded inside MeetingBotsCard so they can be unit-tested in
 * isolation and shared across the split composer components.
 */
import { findMascot } from '../../features/human/Mascot/manifest/manifestService';
import type { MascotManifest } from '../../features/human/Mascot/manifest/types';
import type { MascotColor } from '../../features/human/Mascot/mascotPalette';
import type { ComposioConnection } from '../../lib/composio/types';
import type { BackendMeetJoinInput, MeetingPlatform } from '../../services/meetCallService';
import { composioLogoUrl } from '../composio/toolkitMeta';

/**
 * Mascot ids the meeting-bot backend recognizes. Newer manifest-only mascot
 * ids (e.g. "river-guide") aren't supported there, so the bot falls back to the
 * legacy mascot color for them.
 *
 * "toshi" and "tiny-mascot" are the tinyhumansai/mascots manifest ids that the
 * Recall meeting-bot backend now ships as real mascot assets, so they pass
 * through unchanged instead of being collapsed to a color.
 */
const MEETING_BOT_MASCOT_IDS = new Set([
  'yellow',
  'blue',
  'burgundy',
  'black',
  'navy',
  'toshi',
  'tiny-mascot',
]);

/**
 * Resolve the mascot id to send to the meeting bot: the selected mascot id when
 * the backend recognizes it, otherwise the legacy mascot color (or undefined
 * for `custom`, which has no backend mascot id).
 */
export function resolveMeetingBotMascotId(
  selectedMascotId: string | null,
  mascotColor: MascotColor
): string | undefined {
  if (selectedMascotId && MEETING_BOT_MASCOT_IDS.has(selectedMascotId)) return selectedMascotId;
  if (mascotColor !== 'custom') return mascotColor;
  return undefined;
}

/** Voice/mascot pair the composer selects for the two speaking slots. */
type MeetingMascotVoicePair = {
  primary: { mascotId?: string | null; voiceId?: string };
  secondary?: { mascotId?: string | null; voiceId?: string } | null;
};

/**
 * Build the backend bot's dual-mascot `mascots` array (issue #4277), shared by
 * both join sites — MeetComposer (live join) and UpcomingTable (scheduled join)
 * — so the two paths stay behaviorally identical. They had already begun to
 * drift, which is exactly the class of bug centralising this prevents.
 *
 * Slot 0 (primary) reuses the resolved single-path `mascotId` when the voice
 * pair's primary carries no explicit mascot; slot 1 is the secondary. Slot names
 * come from the manifest so name-addressed routing works in either order ("Hey
 * Toshi" → whichever slot Toshi occupies); the primary falls back to `agentName`
 * only when its manifest entry is unavailable. Per-mascot colors are out of
 * scope — both slots reuse the single `riveColors`.
 *
 * Returns `undefined` (→ the single-mascot `mascotId` join) unless dual mode is
 * on, a secondary is configured, AND both slot ids resolve to a concrete value:
 * a blank slot-0 id would make the backend drop it and render the secondary
 * alone, mismatching the on-camera primary.
 */
export function buildMeetingMascots(input: {
  dualMascotEnabled: boolean;
  mascotVoicePair: MeetingMascotVoicePair;
  manifest: MascotManifest | null;
  mascotId: string | undefined;
  riveColors?: { primaryColor?: string; secondaryColor?: string };
  agentName: string;
}): BackendMeetJoinInput['mascots'] {
  const { dualMascotEnabled, mascotVoicePair, manifest, mascotId, riveColors, agentName } = input;
  const primarySlotId = mascotVoicePair.primary.mascotId ?? mascotId;
  const secondarySlotId = mascotVoicePair.secondary?.mascotId ?? undefined;
  if (!dualMascotEnabled || !mascotVoicePair.secondary || !primarySlotId || !secondarySlotId) {
    return undefined;
  }
  const primaryName =
    (manifest ? findMascot(manifest, primarySlotId)?.name : undefined) ?? agentName;
  const secondaryName = manifest ? findMascot(manifest, secondarySlotId)?.name : undefined;
  return [
    {
      mascotId: primarySlotId,
      name: primaryName,
      voiceId: mascotVoicePair.primary.voiceId,
      riveColors,
    },
    {
      mascotId: secondarySlotId,
      name: secondaryName,
      voiceId: mascotVoicePair.secondary.voiceId,
      riveColors,
    },
  ];
}

// ---------------------------------------------------------------------------
// Platform registry
// ---------------------------------------------------------------------------

/** Ordered list of all supported meeting platforms. */
export const MEETING_PLATFORMS: MeetingPlatform[] = ['gmeet', 'zoom', 'teams', 'webex'];

/**
 * Return the Composio toolkit slug whose connection is the primary identifier
 * for a given platform — used to decide whether a platform chip shows as
 * "connected" or "needs connecting".
 */
export function platformPrimaryToolkit(platform: MeetingPlatform): string {
  switch (platform) {
    case 'gmeet':
      return 'googlemeet';
    case 'zoom':
      return 'zoom';
    case 'teams':
      return 'microsoft_teams';
    case 'webex':
      return 'webex';
  }
}

/**
 * Composio logo CDN URL for a meeting platform.
 * Delegates to the canonical {@link composioLogoUrl} from toolkitMeta.tsx
 * so there is a single source of truth for the logo CDN path.
 */
export function platformLogoUrl(platform: MeetingPlatform): string {
  return composioLogoUrl(platformPrimaryToolkit(platform));
}

/**
 * Return the localised display label for a meeting platform.
 * Delegates to `skills.meetingBots.platforms.*` i18n keys.
 */
export function platformLabel(platform: MeetingPlatform, t: (key: string) => string): string {
  return t(`skills.meetingBots.platforms.${platform}`);
}

/**
 * Return the URL placeholder string for the meeting-link input.
 * Delegates to `skills.meetingBots.platformHints.*` i18n keys.
 */
export function platformUrlPlaceholder(
  platform: MeetingPlatform,
  t: (key: string) => string
): string {
  return t(`skills.meetingBots.platformHints.${platform}`);
}

// ---------------------------------------------------------------------------
// Display-name resolution
// ---------------------------------------------------------------------------

/**
 * Composio only hands back a connected account's email — there is no separate
 * display-name field on `ComposioConnection`. A meeting display name is almost
 * always "First Last" derived from that account, so we best-effort humanize the
 * email's local part (`first.last` → `First Last`).
 */
export function deriveDisplayNameFromEmail(email: string | undefined): string {
  const localPart = email?.split('@')[0]?.trim();
  if (!localPart) return '';
  return localPart
    .split(/[._-]+/)
    .filter(Boolean)
    .map(token => token.charAt(0).toUpperCase() + token.slice(1))
    .join(' ');
}

/**
 * Per-platform priority of Composio toolkits to source the user's in-call
 * display name from: the platform's own connected account first, then the
 * mailbox, then blank. Slugs are canonical Composio slugs (see
 * `canonicalizeComposioToolkitSlug`).
 */
export const NAME_SOURCE_TOOLKITS: Record<MeetingPlatform, string[]> = {
  gmeet: ['googlemeet', 'gmail'],
  zoom: ['zoom', 'gmail'],
  teams: ['microsoft_teams', 'outlook', 'gmail'],
  webex: ['webex', 'gmail'],
};

/**
 * Resolve a default "Your name in this meeting" for the given platform: walk
 * that platform's toolkit priority (own account → mail → blank) and return the
 * first connected account whose email yields a usable name; blank if none are
 * connected. The single entry point the form calls.
 */
export function resolveMeetingDisplayName(
  platform: MeetingPlatform,
  connectionByToolkit: Map<string, ComposioConnection>
): string {
  for (const slug of NAME_SOURCE_TOOLKITS[platform]) {
    const name = deriveDisplayNameFromEmail(connectionByToolkit.get(slug)?.accountEmail);
    if (name) return name;
  }
  return '';
}

/**
 * Infer the meeting platform from a URL's hostname.
 * Returns null when the host doesn't match any known platform or the URL is
 * unparseable.
 *
 * Uses exact-match or proper dot-suffix (e.g. `sub.zoom.us`) to prevent
 * spoofed hosts such as `meet.google.com.attacker.com` from matching.
 */
export function inferPlatformFromUrl(url: string): MeetingPlatform | null {
  let host: string;
  try {
    host = new URL(url).hostname.toLowerCase();
  } catch {
    return null;
  }
  if (host === 'meet.google.com' || host.endsWith('.meet.google.com')) return 'gmeet';
  if (host === 'zoom.us' || host.endsWith('.zoom.us')) return 'zoom';
  if (host === 'teams.microsoft.com' || host.endsWith('.teams.microsoft.com')) return 'teams';
  if (host === 'webex.com' || host.endsWith('.webex.com')) return 'webex';
  return null;
}
