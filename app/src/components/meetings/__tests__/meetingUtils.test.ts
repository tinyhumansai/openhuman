import { describe, expect, it } from 'vitest';

import type { MascotManifest } from '../../../features/human/Mascot/manifest/types';
import type { ComposioConnection } from '../../../lib/composio/types';
import {
  buildMeetingMascots,
  deriveDisplayNameFromEmail,
  inferPlatformFromUrl,
  MEETING_PLATFORMS,
  platformLabel,
  platformPrimaryToolkit,
  platformUrlPlaceholder,
  resolveMeetingBotMascotId,
  resolveMeetingDisplayName,
} from '../meetingUtils';

// ---------------------------------------------------------------------------
// deriveDisplayNameFromEmail
// ---------------------------------------------------------------------------

describe('deriveDisplayNameFromEmail', () => {
  it('converts first.last to First Last', () => {
    expect(deriveDisplayNameFromEmail('first.last@example.com')).toBe('First Last');
  });

  it('handles underscore separator', () => {
    expect(deriveDisplayNameFromEmail('alice_smith@example.com')).toBe('Alice Smith');
  });

  it('handles hyphen separator', () => {
    expect(deriveDisplayNameFromEmail('alice-smith@example.com')).toBe('Alice Smith');
  });

  it('handles single-word local part', () => {
    expect(deriveDisplayNameFromEmail('alice@example.com')).toBe('Alice');
  });

  it('returns empty string for undefined', () => {
    expect(deriveDisplayNameFromEmail(undefined)).toBe('');
  });

  it('returns empty string for empty email', () => {
    expect(deriveDisplayNameFromEmail('')).toBe('');
  });

  it('handles email with no local part', () => {
    expect(deriveDisplayNameFromEmail('@example.com')).toBe('');
  });
});

// ---------------------------------------------------------------------------
// resolveMeetingDisplayName — per-platform priority
// ---------------------------------------------------------------------------

function makeConn(email: string): ComposioConnection {
  return {
    id: `conn-${email}`,
    toolkit: 'unknown',
    status: 'ACTIVE',
    accountEmail: email,
    createdAt: new Date().toISOString(),
  };
}

describe('resolveMeetingDisplayName', () => {
  it('prefers the platform-native toolkit for gmeet over gmail', () => {
    const map = new Map([
      ['googlemeet', makeConn('alice.native@google.com')],
      ['gmail', makeConn('alice.mail@gmail.com')],
    ]);
    expect(resolveMeetingDisplayName('gmeet', map)).toBe('Alice Native');
  });

  it('falls through to gmail for gmeet when platform toolkit missing', () => {
    const map = new Map([['gmail', makeConn('alice.mail@gmail.com')]]);
    expect(resolveMeetingDisplayName('gmeet', map)).toBe('Alice Mail');
  });

  it('prefers zoom over gmail', () => {
    const map = new Map([
      ['zoom', makeConn('bob.zoom@company.com')],
      ['gmail', makeConn('bob.gmail@gmail.com')],
    ]);
    expect(resolveMeetingDisplayName('zoom', map)).toBe('Bob Zoom');
  });

  it('prefers microsoft_teams over outlook and gmail for teams', () => {
    const map = new Map([
      ['microsoft_teams', makeConn('carol.teams@company.com')],
      ['outlook', makeConn('carol.outlook@company.com')],
      ['gmail', makeConn('carol.gmail@gmail.com')],
    ]);
    expect(resolveMeetingDisplayName('teams', map)).toBe('Carol Teams');
  });

  it('falls through to outlook for teams when ms_teams missing', () => {
    const map = new Map([
      ['outlook', makeConn('carol.outlook@company.com')],
      ['gmail', makeConn('carol.gmail@gmail.com')],
    ]);
    expect(resolveMeetingDisplayName('teams', map)).toBe('Carol Outlook');
  });

  it('returns blank when no toolkits are connected', () => {
    expect(resolveMeetingDisplayName('gmeet', new Map())).toBe('');
  });

  it('skips entries whose email yields an empty name', () => {
    const map = new Map([
      ['googlemeet', makeConn('@no-local-part.com')],
      ['gmail', makeConn('alice.gmail@gmail.com')],
    ]);
    expect(resolveMeetingDisplayName('gmeet', map)).toBe('Alice Gmail');
  });

  it('prefers webex over gmail for webex platform', () => {
    const map = new Map([
      ['webex', makeConn('dave.webex@cisco.com')],
      ['gmail', makeConn('dave.gmail@gmail.com')],
    ]);
    expect(resolveMeetingDisplayName('webex', map)).toBe('Dave Webex');
  });
});

// ---------------------------------------------------------------------------
// MEETING_PLATFORMS
// ---------------------------------------------------------------------------

describe('MEETING_PLATFORMS', () => {
  it('includes all four platforms', () => {
    expect(MEETING_PLATFORMS).toEqual(expect.arrayContaining(['gmeet', 'zoom', 'teams', 'webex']));
    expect(MEETING_PLATFORMS).toHaveLength(4);
  });
});

// ---------------------------------------------------------------------------
// platformPrimaryToolkit
// ---------------------------------------------------------------------------

describe('platformPrimaryToolkit', () => {
  it('returns googlemeet for gmeet', () => {
    expect(platformPrimaryToolkit('gmeet')).toBe('googlemeet');
  });

  it('returns zoom for zoom', () => {
    expect(platformPrimaryToolkit('zoom')).toBe('zoom');
  });

  it('returns microsoft_teams for teams', () => {
    expect(platformPrimaryToolkit('teams')).toBe('microsoft_teams');
  });

  it('returns webex for webex', () => {
    expect(platformPrimaryToolkit('webex')).toBe('webex');
  });
});

// ---------------------------------------------------------------------------
// platformLabel / platformUrlPlaceholder
// ---------------------------------------------------------------------------

describe('platformLabel', () => {
  const t = (key: string) => {
    const translations: Record<string, string> = {
      'skills.meetingBots.platforms.gmeet': 'Google Meet',
      'skills.meetingBots.platforms.zoom': 'Zoom',
      'skills.meetingBots.platforms.teams': 'Microsoft Teams',
      'skills.meetingBots.platforms.webex': 'Webex',
    };
    return translations[key] ?? key;
  };

  it('returns Google Meet for gmeet', () => {
    expect(platformLabel('gmeet', t)).toBe('Google Meet');
  });

  it('returns Webex for webex', () => {
    expect(platformLabel('webex', t)).toBe('Webex');
  });
});

describe('platformUrlPlaceholder', () => {
  const t = (key: string) => {
    const translations: Record<string, string> = {
      'skills.meetingBots.platformHints.gmeet': 'meet.google.com/abc-defg-hij',
      'skills.meetingBots.platformHints.zoom': 'zoom.us/j/...',
      'skills.meetingBots.platformHints.teams': 'teams.microsoft.com/...',
      'skills.meetingBots.platformHints.webex': 'webex.com/meet/...',
    };
    return translations[key] ?? key;
  };

  it('returns the gmeet URL hint', () => {
    expect(platformUrlPlaceholder('gmeet', t)).toBe('meet.google.com/abc-defg-hij');
  });

  it('returns the webex URL hint', () => {
    expect(platformUrlPlaceholder('webex', t)).toBe('webex.com/meet/...');
  });
});

// ---------------------------------------------------------------------------
// inferPlatformFromUrl — strict host matching (#6 security fix)
// ---------------------------------------------------------------------------

describe('inferPlatformFromUrl', () => {
  it('returns gmeet for a standard Google Meet URL', () => {
    expect(inferPlatformFromUrl('https://meet.google.com/abc-def-ghi')).toBe('gmeet');
  });

  it('returns gmeet for a subdomain of meet.google.com', () => {
    expect(inferPlatformFromUrl('https://sub.meet.google.com/abc-def-ghi')).toBe('gmeet');
  });

  it('rejects a host that contains meet.google.com as a suffix of a label (spoofed)', () => {
    // meet.google.com.attacker.com must NOT match
    expect(inferPlatformFromUrl('https://meet.google.com.attacker.com/abc')).toBeNull();
  });

  it('returns zoom for zoom.us', () => {
    expect(inferPlatformFromUrl('https://zoom.us/j/123456')).toBe('zoom');
  });

  it('returns zoom for a subdomain of zoom.us (e.g. my.zoom.us)', () => {
    expect(inferPlatformFromUrl('https://my.zoom.us/j/123')).toBe('zoom');
  });

  it('rejects a host that ends in a word that happens to contain zoom.us', () => {
    expect(inferPlatformFromUrl('https://evil-zoom.us/j/123')).toBeNull();
  });

  it('returns teams for teams.microsoft.com', () => {
    expect(inferPlatformFromUrl('https://teams.microsoft.com/l/meetup-join/123')).toBe('teams');
  });

  it('rejects a spoofed teams host', () => {
    expect(inferPlatformFromUrl('https://teams.microsoft.com.evil.org/meeting')).toBeNull();
  });

  it('returns webex for webex.com', () => {
    expect(inferPlatformFromUrl('https://webex.com/meet/abc')).toBe('webex');
  });

  it('returns webex for a subdomain of webex.com', () => {
    expect(inferPlatformFromUrl('https://cisco.webex.com/meet/abc')).toBe('webex');
  });

  it('returns null for an unrecognized URL', () => {
    expect(inferPlatformFromUrl('https://example.com/meeting')).toBeNull();
  });

  it('returns null for an invalid (unparseable) URL string', () => {
    expect(inferPlatformFromUrl('not-a-url')).toBeNull();
  });
});

describe('resolveMeetingBotMascotId', () => {
  it('keeps a selected mascot id the backend recognizes', () => {
    expect(resolveMeetingBotMascotId('navy', 'yellow')).toBe('navy');
  });

  it('keeps the "toshi" manifest mascot id the backend now ships as an asset', () => {
    expect(resolveMeetingBotMascotId('toshi', 'yellow')).toBe('toshi');
  });

  it('keeps the "tiny-mascot" manifest mascot id the backend now ships as an asset', () => {
    expect(resolveMeetingBotMascotId('tiny-mascot', 'navy')).toBe('tiny-mascot');
  });

  it('falls back to the legacy mascot color for a manifest-only mascot id the backend still lacks', () => {
    expect(resolveMeetingBotMascotId('jarvis', 'yellow')).toBe('yellow');
  });

  it('uses the mascot color when no mascot id is selected', () => {
    expect(resolveMeetingBotMascotId(null, 'burgundy')).toBe('burgundy');
  });

  it('returns undefined for custom color with an unrecognized mascot id', () => {
    expect(resolveMeetingBotMascotId('river-guide', 'custom')).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// buildMeetingMascots
// ---------------------------------------------------------------------------

describe('buildMeetingMascots', () => {
  // findMascot only reads `.id`/`.name`, so a partial manifest is enough.
  const manifest = {
    mascots: [
      { id: 'toshi', name: 'Toshi' },
      { id: 'tiny-mascot', name: 'Tiny' },
    ],
  } as unknown as MascotManifest;

  const pair = {
    primary: { mascotId: 'toshi', voiceId: 'voice-a' },
    secondary: { mascotId: 'tiny-mascot', voiceId: 'voice-b' },
  };

  it('returns undefined when dual mode is off', () => {
    expect(
      buildMeetingMascots({
        dualMascotEnabled: false,
        mascotVoicePair: pair,
        manifest,
        mascotId: 'toshi',
        agentName: 'Tiny',
      })
    ).toBeUndefined();
  });

  it('returns undefined when no secondary is configured', () => {
    expect(
      buildMeetingMascots({
        dualMascotEnabled: true,
        mascotVoicePair: { primary: { mascotId: 'toshi', voiceId: 'voice-a' } },
        manifest,
        mascotId: 'toshi',
        agentName: 'Tiny',
      })
    ).toBeUndefined();
  });

  it('returns undefined when the primary slot id cannot be resolved', () => {
    // primary has no explicit mascot AND the resolved single-path id is undefined.
    expect(
      buildMeetingMascots({
        dualMascotEnabled: true,
        mascotVoicePair: {
          primary: { voiceId: 'voice-a' },
          secondary: { mascotId: 'tiny-mascot', voiceId: 'voice-b' },
        },
        manifest,
        mascotId: undefined,
        agentName: 'Tiny',
      })
    ).toBeUndefined();
  });

  it('falls back to the resolved single-path mascotId for the primary slot', () => {
    const mascots = buildMeetingMascots({
      dualMascotEnabled: true,
      mascotVoicePair: {
        primary: { voiceId: 'voice-a' },
        secondary: { mascotId: 'tiny-mascot', voiceId: 'voice-b' },
      },
      manifest,
      mascotId: 'toshi',
      agentName: 'Tiny',
    });
    expect(mascots?.[0].mascotId).toBe('toshi');
  });

  it('builds two slots with manifest names and per-slot voices', () => {
    const riveColors = { primaryColor: '#111', secondaryColor: '#222' };
    const mascots = buildMeetingMascots({
      dualMascotEnabled: true,
      mascotVoicePair: pair,
      manifest,
      mascotId: 'toshi',
      riveColors,
      agentName: 'Tiny',
    });
    expect(mascots).toEqual([
      { mascotId: 'toshi', name: 'Toshi', voiceId: 'voice-a', riveColors },
      { mascotId: 'tiny-mascot', name: 'Tiny', voiceId: 'voice-b', riveColors },
    ]);
  });

  it('resolves names by slot so either mascot order works', () => {
    const mascots = buildMeetingMascots({
      dualMascotEnabled: true,
      mascotVoicePair: {
        primary: { mascotId: 'tiny-mascot', voiceId: 'voice-b' },
        secondary: { mascotId: 'toshi', voiceId: 'voice-a' },
      },
      manifest,
      mascotId: 'tiny-mascot',
      agentName: 'Tiny',
    });
    expect(mascots?.map(m => m.name)).toEqual(['Tiny', 'Toshi']);
  });

  it('falls back to agentName for the primary when its manifest entry is missing', () => {
    const mascots = buildMeetingMascots({
      dualMascotEnabled: true,
      mascotVoicePair: {
        primary: { mascotId: 'river-guide', voiceId: 'voice-a' },
        secondary: { mascotId: 'toshi', voiceId: 'voice-a' },
      },
      manifest,
      mascotId: 'river-guide',
      agentName: 'Persona',
    });
    expect(mascots?.[0].name).toBe('Persona');
    expect(mascots?.[1].name).toBe('Toshi');
  });
});
