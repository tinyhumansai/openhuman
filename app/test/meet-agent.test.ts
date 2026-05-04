/**
 * Tests for app/src-tauri/recipes/google-meet/agent.js pure helpers.
 *
 * We load the agent script in a fresh jsdom window via Function() evaluation,
 * set up the role="agent" context, and extract `window.__openhumanMeetAgent.pure`
 * for unit testing. The polling loop is never started in this environment because
 * we don't provide a real meetingUrl navigation context.
 */
import fs from 'fs';
import path from 'path';
import { beforeEach, describe, expect, it } from 'vitest';

// Read the agent.js source once.
const AGENT_JS_PATH = path.resolve(__dirname, '../src-tauri/recipes/google-meet/agent.js');
const agentSource = fs.readFileSync(AGENT_JS_PATH, 'utf8');

/**
 * Evaluate agent.js in the current jsdom window with a mock context and API.
 * Returns the `pure` namespace from `window.__openhumanMeetAgent`.
 */
function loadAgent(meetingUrl = 'https://meet.google.com/abc-defg-hij') {
  // Reset any prior agent state.
  delete (window as Window & { __openhumanMeetAgent?: unknown }).__openhumanMeetAgent;

  // Set up the recipe context with role="agent".
  (window as Window & { __OPENHUMAN_RECIPE_CTX__?: unknown }).__OPENHUMAN_RECIPE_CTX__ = {
    accountId: 'test-account',
    provider: 'google-meet',
    role: 'agent',
    meetingUrl,
  };

  // Minimal mock of the runtime API (emit + log).
  const emitted: Array<{ kind: string; payload: unknown }> = [];
  (window as Window & { __openhumanRecipe?: unknown }).__openhumanRecipe = {
    emit: (kind: string, payload: unknown) => emitted.push({ kind, payload }),
    log: () => {},
    loop: () => {},
  };

  // Run the agent script.
  // Function constructor used intentionally to evaluate the agent script in the jsdom context.
  // biome-ignore lint: intentional use of Function constructor for test harness
  new Function(agentSource)();

  const agent = (window as Window & { __openhumanMeetAgent?: { pure: Record<string, unknown> } })
    .__openhumanMeetAgent;

  if (!agent) throw new Error('__openhumanMeetAgent not set after loading agent.js');

  return agent.pure as {
    extractMeetingCode: (href: string) => string | null;
    findJoinButton: (doc: Document) => Element | null;
    findMicButton: (doc: Document) => Element | null;
    findCamButton: (doc: Document) => Element | null;
    isMicOn: (btn: Element | null) => boolean;
    isCamOn: (btn: Element | null) => boolean;
    isInCall: (doc: Document) => boolean;
    findLeaveButton: (doc: Document) => Element | null;
    isUnjoinableScreen: (doc: Document) => string | null;
  };
}

// ─── extractMeetingCode ────────────────────────────────────────────────────

describe('extractMeetingCode', () => {
  let pure: ReturnType<typeof loadAgent>;

  beforeEach(() => {
    pure = loadAgent();
  });

  it('extracts a standard 3-part code', () => {
    expect(pure.extractMeetingCode('https://meet.google.com/abc-defg-hij')).toBe('abc-defg-hij');
  });

  it('extracts code with trailing slash', () => {
    expect(pure.extractMeetingCode('https://meet.google.com/abc-defg-hij/')).toBe('abc-defg-hij');
  });

  it('extracts code with query string', () => {
    expect(pure.extractMeetingCode('https://meet.google.com/abc-defg-hij?authuser=0')).toBe(
      'abc-defg-hij'
    );
  });

  it('returns null for a non-meeting URL', () => {
    expect(pure.extractMeetingCode('https://meet.google.com/')).toBeNull();
  });

  it('returns null for empty string', () => {
    expect(pure.extractMeetingCode('')).toBeNull();
  });

  it('returns null for a URL with no matching pathname', () => {
    expect(pure.extractMeetingCode('https://meet.google.com/settings')).toBeNull();
  });
});

// ─── findJoinButton ────────────────────────────────────────────────────────

describe('findJoinButton', () => {
  let pure: ReturnType<typeof loadAgent>;

  beforeEach(() => {
    pure = loadAgent();
    document.body.innerHTML = '';
  });

  it('finds button by jsname', () => {
    document.body.innerHTML = '<button jsname="Qx7uuf">Join now</button>';
    expect(pure.findJoinButton(document)).not.toBeNull();
  });

  it('finds button by aria-label "Join now"', () => {
    document.body.innerHTML = '<button aria-label="Join now">Join now</button>';
    expect(pure.findJoinButton(document)).not.toBeNull();
  });

  it('finds button by aria-label "Ask to join"', () => {
    document.body.innerHTML = '<button aria-label="Ask to join">Ask to join</button>';
    expect(pure.findJoinButton(document)).not.toBeNull();
  });

  it('finds button by text content fallback', () => {
    document.body.innerHTML = '<button>Join now</button>';
    expect(pure.findJoinButton(document)).not.toBeNull();
  });

  it('finds button by text "Ask to join" fallback', () => {
    document.body.innerHTML = '<button>Ask to join</button>';
    expect(pure.findJoinButton(document)).not.toBeNull();
  });

  it('returns null when only a disabled button is present', () => {
    document.body.innerHTML = '<button jsname="Qx7uuf" disabled>Join now</button>';
    expect(pure.findJoinButton(document)).toBeNull();
  });

  it('returns null when no matching button exists', () => {
    document.body.innerHTML = '<button>Settings</button>';
    expect(pure.findJoinButton(document)).toBeNull();
  });
});

// ─── findMicButton / isMicOn ───────────────────────────────────────────────

describe('findMicButton + isMicOn', () => {
  let pure: ReturnType<typeof loadAgent>;

  beforeEach(() => {
    pure = loadAgent();
    document.body.innerHTML = '';
  });

  it('finds mic button by aria-label containing "microphone"', () => {
    document.body.innerHTML = '<div role="button" aria-label="Turn off microphone">mic</div>';
    expect(pure.findMicButton(document)).not.toBeNull();
  });

  it('isMicOn defaults to false on an ambiguous node', () => {
    document.body.innerHTML = '<div role="button" aria-label="microphone">mic</div>';
    const btn = pure.findMicButton(document);
    expect(pure.isMicOn(btn)).toBe(false);
  });

  it('isMicOn returns true when aria-pressed="true"', () => {
    document.body.innerHTML =
      '<div role="button" aria-label="microphone" aria-pressed="true">mic</div>';
    const btn = pure.findMicButton(document);
    expect(pure.isMicOn(btn)).toBe(true);
  });

  it('isMicOn returns true when data-is-muted="false"', () => {
    document.body.innerHTML =
      '<div role="button" data-is-muted="false" aria-label="microphone">mic</div>';
    const btn = pure.findMicButton(document);
    expect(pure.isMicOn(btn)).toBe(true);
  });

  it('isMicOn returns false for null', () => {
    expect(pure.isMicOn(null)).toBe(false);
  });
});

// ─── isInCall ──────────────────────────────────────────────────────────────

describe('isInCall', () => {
  let pure: ReturnType<typeof loadAgent>;

  beforeEach(() => {
    pure = loadAgent();
    document.body.innerHTML = '';
  });

  it('returns true when data-self-name is present', () => {
    document.body.innerHTML = '<div data-self-name="Alice"></div>';
    expect(pure.isInCall(document)).toBe(true);
  });

  it('returns true when data-participant-id is present', () => {
    document.body.innerHTML = '<div data-participant-id="part-123"></div>';
    expect(pure.isInCall(document)).toBe(true);
  });

  it('returns false when neither signal is present', () => {
    document.body.innerHTML = '<div>Lobby</div>';
    expect(pure.isInCall(document)).toBe(false);
  });
});

// ─── isUnjoinableScreen ────────────────────────────────────────────────────

describe('isUnjoinableScreen', () => {
  let pure: ReturnType<typeof loadAgent>;

  beforeEach(() => {
    pure = loadAgent();
    document.body.innerHTML = '';
  });

  it('returns "meeting-not-found" for "Check your meeting code" text', () => {
    document.body.innerHTML = '<p>Check your meeting code and try again.</p>';
    expect(pure.isUnjoinableScreen(document)).toBe('meeting-not-found');
  });

  it('returns "permission-denied" for "You can\'t join this video call" text', () => {
    document.body.innerHTML = "<p>You can't join this video call.</p>";
    expect(pure.isUnjoinableScreen(document)).toBe('permission-denied');
  });

  it('returns "sign-in-required" when both "Switch account" and "sign in" are present', () => {
    document.body.innerHTML = '<p>Switch account or sign in to continue.</p>';
    expect(pure.isUnjoinableScreen(document)).toBe('sign-in-required');
  });

  it('returns null when none of the patterns match', () => {
    document.body.innerHTML = '<div>Joining meeting...</div>';
    expect(pure.isUnjoinableScreen(document)).toBeNull();
  });
});

// ─── findLeaveButton ───────────────────────────────────────────────────────

describe('findLeaveButton', () => {
  let pure: ReturnType<typeof loadAgent>;

  beforeEach(() => {
    pure = loadAgent();
    document.body.innerHTML = '';
  });

  it('finds button by aria-label "Leave call"', () => {
    document.body.innerHTML = '<button aria-label="Leave call">Leave</button>';
    expect(pure.findLeaveButton(document)).not.toBeNull();
  });

  it('finds button by jsname CQylAd', () => {
    document.body.innerHTML = '<div jsname="CQylAd">Leave</div>';
    expect(pure.findLeaveButton(document)).not.toBeNull();
  });

  it('finds button by text content fallback', () => {
    document.body.innerHTML = '<button>Leave call</button>';
    expect(pure.findLeaveButton(document)).not.toBeNull();
  });

  it('returns null when no leave button present', () => {
    document.body.innerHTML = '<button>Join now</button>';
    expect(pure.findLeaveButton(document)).toBeNull();
  });
});

// ─── Role gate ─────────────────────────────────────────────────────────────

describe('role gate', () => {
  it('does not set __openhumanMeetAgent when role is not "agent"', () => {
    // @ts-expect-error - test harness
    delete (window as Window).__openhumanMeetAgent;

    (window as Window & { __OPENHUMAN_RECIPE_CTX__?: unknown }).__OPENHUMAN_RECIPE_CTX__ = {
      accountId: 'test-account',
      provider: 'google-meet',
      role: 'user', // NOT agent
      meetingUrl: 'https://meet.google.com/abc-defg-hij',
    };

    // Function constructor used intentionally for test harness evaluation.
    // biome-ignore lint: intentional use of Function constructor for test harness
    new Function(agentSource)();

    expect(
      (window as Window & { __openhumanMeetAgent?: unknown }).__openhumanMeetAgent
    ).toBeUndefined();
  });
});
