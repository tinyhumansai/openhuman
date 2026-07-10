import { invoke, isTauri } from '@tauri-apps/api/core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../coreRpcClient';
import {
  closeMeetCall,
  getEventPolicies,
  joinMeetCall,
  joinMeetViaBackendBot,
  listMeetCalls,
  listUpcomingMeetings,
  parseTranscriptLine,
  setEventPolicy,
} from '../meetCallService';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: vi.fn() }));

vi.mock('../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

describe('joinMeetCall', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(isTauri).mockReturnValue(true);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('rejects empty inputs without contacting the core', async () => {
    await expect(joinMeetCall({ meetUrl: '   ', displayName: 'Alice' })).rejects.toThrow(
      /Meet link/i
    );
    await expect(
      joinMeetCall({ meetUrl: 'https://meet.google.com/abc-defg-hij', displayName: '' })
    ).rejects.toThrow(/display name/i);
    expect(callCoreRpc).not.toHaveBeenCalled();
    expect(invoke).not.toHaveBeenCalled();
  });

  it('chains the core RPC into the Tauri window-open command', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      ok: true,
      request_id: 'req-1',
      meet_url: 'https://meet.google.com/abc-defg-hij',
      display_name: 'Agent Alice',
    } as never);
    vi.mocked(invoke).mockResolvedValueOnce('meet-call-req-1');

    const result = await joinMeetCall({
      meetUrl: 'https://meet.google.com/abc-defg-hij',
      displayName: 'Agent Alice',
      ownerDisplayName: 'Owner Bob',
    });

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.meet_join_call',
      params: { meet_url: 'https://meet.google.com/abc-defg-hij', display_name: 'Agent Alice' },
    });
    // owner_display_name is forwarded to the shell (not to the core's
    // meet_join_call, which is stateless validation only) — assert on
    // the shell args, not the core RPC params.
    expect(invoke).toHaveBeenCalledWith('meet_call_open_window', {
      args: {
        request_id: 'req-1',
        meet_url: 'https://meet.google.com/abc-defg-hij',
        display_name: 'Agent Alice',
        owner_display_name: 'Owner Bob',
      },
    });
    expect(result).toEqual({
      requestId: 'req-1',
      meetUrl: 'https://meet.google.com/abc-defg-hij',
      displayName: 'Agent Alice',
      ownerDisplayName: 'Owner Bob',
      windowLabel: 'meet-call-req-1',
    });
  });

  it('forwards per-mascot voices to the shell when provided (issue #4277)', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      ok: true,
      request_id: 'req-2',
      meet_url: 'https://meet.google.com/abc-defg-hij',
      display_name: 'Agent Alice',
    } as never);
    vi.mocked(invoke).mockResolvedValueOnce('meet-call-req-2');

    await joinMeetCall({
      meetUrl: 'https://meet.google.com/abc-defg-hij',
      displayName: 'Agent Alice',
      ownerDisplayName: 'Owner Bob',
      primaryVoiceId: ' voice-a ',
      secondaryVoiceId: 'voice-b',
    });

    expect(invoke).toHaveBeenCalledWith('meet_call_open_window', {
      args: {
        request_id: 'req-2',
        meet_url: 'https://meet.google.com/abc-defg-hij',
        display_name: 'Agent Alice',
        owner_display_name: 'Owner Bob',
        primary_voice_id: 'voice-a',
        secondary_voice_id: 'voice-b',
      },
    });
  });

  it('throws if core rejects the request', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ ok: false } as never);
    await expect(
      joinMeetCall({
        meetUrl: 'https://meet.google.com/abc-defg-hij',
        displayName: 'Agent Alice',
        ownerDisplayName: 'Owner Bob',
      })
    ).rejects.toThrow(/Core rejected/);
    expect(invoke).not.toHaveBeenCalled();
  });

  it('refuses to open a window outside the desktop shell', async () => {
    vi.mocked(isTauri).mockReturnValue(false);
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      ok: true,
      request_id: 'req-1',
      meet_url: 'https://meet.google.com/abc-defg-hij',
      display_name: 'Agent Alice',
    } as never);

    await expect(
      joinMeetCall({
        meetUrl: 'https://meet.google.com/abc-defg-hij',
        displayName: 'Agent Alice',
        ownerDisplayName: 'Owner Bob',
      })
    ).rejects.toThrow(/desktop app/);
    expect(invoke).not.toHaveBeenCalled();
  });

  it('rejects an empty owner_display_name as a privacy-lock guard', async () => {
    // Privacy lock: empty owner would fail closed at the core wake
    // gate (no captions ever wake the bot). Surface the requirement
    // up front so the user doesn't sit through a join only to find
    // the bot silent — see feat/mascot-meet-flowA Plan C.
    await expect(
      joinMeetCall({
        meetUrl: 'https://meet.google.com/abc-defg-hij',
        displayName: 'Agent Alice',
        ownerDisplayName: '   ',
      })
    ).rejects.toThrow(/your own name/i);
    expect(callCoreRpc).not.toHaveBeenCalled();
    expect(invoke).not.toHaveBeenCalled();
  });
});

describe('listMeetCalls', () => {
  beforeEach(() => {
    // Use mockReset (not just clearAllMocks) to drain any once-queues
    // left over from the joinMeetCall describe block above, ensuring
    // each test below starts with a fresh callCoreRpc mock.
    vi.mocked(callCoreRpc).mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('returns the calls array from a successful core response', async () => {
    const mockCalls = [
      {
        request_id: 'req-1',
        meet_url: 'https://meet.google.com/abc-defg-hij',
        bot_display_name: 'OpenHuman',
        owner_display_name: 'Alice',
        started_at_ms: 1700000000000,
        ended_at_ms: 1700000060000,
        listened_seconds: 30,
        spoken_seconds: 30,
        turn_count: 3,
      },
    ];
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ ok: true, calls: mockCalls, count: 1 } as never);

    const result = await listMeetCalls(20);

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.meet_agent_list_calls',
      params: { limit: 20 },
    });
    expect(result).toEqual(mockCalls);
  });

  it('returns an empty array when the core response has no calls field', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ ok: true, calls: undefined, count: 0 } as never);

    const result = await listMeetCalls(10);

    expect(result).toEqual([]);
  });

  it('throws when the core responds with ok: false', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ ok: false } as never);

    await expect(listMeetCalls(20)).rejects.toThrow(/meet_agent_list_calls/);
  });

  it('throws when the core responds with a falsy result', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce(null as never);

    await expect(listMeetCalls(20)).rejects.toThrow(/meet_agent_list_calls/);
  });

  it('uses the default limit of 20 when no argument is provided', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ ok: true, calls: [], count: 0 } as never);

    await listMeetCalls();

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.meet_agent_list_calls',
      params: { limit: 20 },
    });
  });
});

describe('joinMeetViaBackendBot', () => {
  beforeEach(() => {
    vi.mocked(callCoreRpc).mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('emits the backend Recall bot join RPC with camelCase colors', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      ok: true,
      meet_url: 'https://meet.google.com/abc-defg-hij',
      platform: 'gmeet',
    } as never);

    const result = await joinMeetViaBackendBot({
      meetUrl: ' https://meet.google.com/abc-defg-hij ',
      displayName: 'OpenHuman',
      platform: 'gmeet',
      agentName: 'OpenHuman',
      systemPrompt: 'Answer only when addressed.',
      riveColors: { primaryColor: '#ffcc00', secondaryColor: '#112233' },
    });

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.agent_meetings_join',
      params: {
        meet_url: 'https://meet.google.com/abc-defg-hij',
        display_name: 'OpenHuman',
        platform: 'gmeet',
        agent_name: 'OpenHuman',
        system_prompt: 'Answer only when addressed.',
        rive_colors: { primary_color: '#ffcc00', secondary_color: '#112233' },
      },
    });
    expect(result).toEqual({ meetUrl: 'https://meet.google.com/abc-defg-hij', platform: 'gmeet' });
  });

  it('omits mascots[] for a single-mascot join (unchanged wire shape)', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      ok: true,
      meet_url: 'https://meet.google.com/abc-defg-hij',
      platform: 'gmeet',
    } as never);

    await joinMeetViaBackendBot({
      meetUrl: 'https://meet.google.com/abc-defg-hij',
      mascotId: 'yellow',
    });

    // No `mascots` key on the params → the single `mascot_id` path is
    // byte-identical to before (undefined is dropped by the RPC boundary).
    const params = vi.mocked(callCoreRpc).mock.calls[0][0].params as Record<string, unknown>;
    expect(params.mascots).toBeUndefined();
    expect(params.mascot_id).toBe('yellow');
  });

  it('maps dual mascots[] to snake_case slots and drops blank ids (issue #4277)', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      ok: true,
      meet_url: 'https://meet.google.com/abc-defg-hij',
      platform: 'gmeet',
    } as never);

    await joinMeetViaBackendBot({
      meetUrl: 'https://meet.google.com/abc-defg-hij',
      mascots: [
        {
          mascotId: ' tiny-mascot ',
          name: ' Tiny ',
          voiceId: ' voice-a ',
          riveColors: { primaryColor: '#111', secondaryColor: '#222' },
        },
        { mascotId: 'toshi', voiceId: 'voice-b' },
        { mascotId: '   ', voiceId: 'ignored' },
      ],
    });

    const params = vi.mocked(callCoreRpc).mock.calls[0][0].params as Record<string, unknown>;
    expect(params.mascots).toEqual([
      {
        mascot_id: 'tiny-mascot',
        // Name-addressed routing (#4277 follow-up): trimmed + forwarded.
        name: 'Tiny',
        voice_id: 'voice-a',
        rive_colors: { primary_color: '#111', secondary_color: '#222' },
      },
      // Slot 1 supplies no name → `name` omitted (undefined).
      { mascot_id: 'toshi', name: undefined, voice_id: 'voice-b', rive_colors: undefined },
    ]);
  });

  it('rejects an empty meeting link before contacting core', async () => {
    await expect(joinMeetViaBackendBot({ meetUrl: '   ' })).rejects.toThrow(/meeting link/i);
    expect(callCoreRpc).not.toHaveBeenCalled();
  });
});

describe('closeMeetCall', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('forwards the request_id and returns the shell result', async () => {
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(invoke).mockResolvedValueOnce(true);

    await expect(closeMeetCall('req-1')).resolves.toBe(true);
    expect(invoke).toHaveBeenCalledWith('meet_call_close_window', { requestId: 'req-1' });
  });

  it('is a no-op outside the desktop shell', async () => {
    vi.mocked(isTauri).mockReturnValue(false);

    await expect(closeMeetCall('req-1')).resolves.toBe(false);
    expect(invoke).not.toHaveBeenCalled();
  });
});

describe('listUpcomingMeetings', () => {
  beforeEach(() => {
    vi.mocked(callCoreRpc).mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  const mockMeeting = {
    calendar_event_id: 'evt-1',
    title: 'Standup',
    start_time_ms: Date.now() + 30 * 60 * 1000,
    end_time_ms: Date.now() + 60 * 60 * 1000,
    meet_url: 'https://meet.google.com/abc-def-ghi',
    platform: 'gmeet',
    participant_count: 4,
    organizer: 'alice@example.com',
    join_policy: 'ask',
    calendar_source: 'google:alice@example.com',
  };

  it('calls openhuman.meet_list_upcoming with no params when no args given', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ ok: true, meetings: [mockMeeting] } as never);

    const result = await listUpcomingMeetings();

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.meet_list_upcoming',
      params: {},
    });
    expect(result).toEqual([mockMeeting]);
  });

  it('forwards lookahead_minutes and limit when provided', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ ok: true, meetings: [] } as never);

    await listUpcomingMeetings(120, 10);

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.meet_list_upcoming',
      params: { lookahead_minutes: 120, limit: 10 },
    });
  });

  it('returns an empty array when core returns no meetings', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ ok: true, meetings: undefined } as never);

    const result = await listUpcomingMeetings();
    expect(result).toEqual([]);
  });

  it('throws when core returns ok: false', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ ok: false } as never);
    await expect(listUpcomingMeetings()).rejects.toThrow(/meet_list_upcoming/);
  });

  it('throws when core returns a falsy result', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce(null as never);
    await expect(listUpcomingMeetings()).rejects.toThrow(/meet_list_upcoming/);
  });
});

describe('setEventPolicy', () => {
  beforeEach(() => {
    vi.mocked(callCoreRpc).mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('calls meet_set_event_policy with correct params', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ ok: true } as never);
    await setEventPolicy('evt-123', 'skip');
    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.meet_set_event_policy',
      params: { calendar_event_id: 'evt-123', policy: 'skip' },
    });
  });

  it('throws when core returns ok=false', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ ok: false } as never);
    await expect(setEventPolicy('evt-123', 'auto')).rejects.toThrow('meet_set_event_policy');
  });
});

describe('getEventPolicies', () => {
  beforeEach(() => {
    vi.mocked(callCoreRpc).mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('calls meet_get_event_policies with correct params', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      ok: true,
      policies: { 'evt-1': 'skip' },
    } as never);
    const result = await getEventPolicies(['evt-1', 'evt-2']);
    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.meet_get_event_policies',
      params: { calendar_event_ids: ['evt-1', 'evt-2'] },
    });
    expect(result).toEqual({ 'evt-1': 'skip' });
  });

  it('returns empty object for empty input', async () => {
    const result = await getEventPolicies([]);
    expect(callCoreRpc).not.toHaveBeenCalled();
    expect(result).toEqual({});
  });

  it('throws when core returns ok=false', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ ok: false } as never);
    await expect(getEventPolicies(['evt-x'])).rejects.toThrow('meet_get_event_policies');
  });
});

describe('parseTranscriptLine', () => {
  it('parses a line with a [MM:SS] [Name] prefix', () => {
    const result = parseTranscriptLine({
      role: 'participant',
      content: '[1:23] [Alice] Hello there!',
    });
    expect(result.timestamp).toBe('1:23');
    expect(result.speaker).toBe('Alice');
    expect(result.text).toBe('Hello there!');
    expect(result.role).toBe('participant');
  });

  it('returns null timestamp and speaker when prefix is absent', () => {
    const result = parseTranscriptLine({ role: 'assistant', content: 'How can I help?' });
    expect(result.timestamp).toBeNull();
    expect(result.speaker).toBeNull();
    expect(result.text).toBe('How can I help?');
    expect(result.role).toBe('assistant');
  });

  it('handles partial brackets — no match, returns full content', () => {
    const result = parseTranscriptLine({
      role: 'participant',
      content: '[1:23] missing second bracket',
    });
    expect(result.timestamp).toBeNull();
    expect(result.speaker).toBeNull();
    expect(result.text).toBe('[1:23] missing second bracket');
  });

  it('handles malformed content gracefully', () => {
    const result = parseTranscriptLine({ role: 'participant', content: 'just plain text' });
    expect(result.timestamp).toBeNull();
    expect(result.speaker).toBeNull();
    expect(result.text).toBe('just plain text');
  });
});
