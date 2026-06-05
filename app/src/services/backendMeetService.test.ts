import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from './coreRpcClient';
import { joinMeetViaBackendBot, leaveBackendMeetBot, sendHarnessResponse } from './meetCallService';

vi.mock('./coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const mockCallCoreRpc = vi.mocked(callCoreRpc);

beforeEach(() => {
  vi.resetAllMocks();
});

describe('joinMeetViaBackendBot', () => {
  it('calls agent_meetings_join with all params (including new customization fields)', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      ok: true,
      meet_url: 'https://meet.google.com/abc-defg-hij',
      platform: 'gmeet',
    });

    const result = await joinMeetViaBackendBot({ meetUrl: 'https://meet.google.com/abc-defg-hij' });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.agent_meetings_join',
      params: {
        meet_url: 'https://meet.google.com/abc-defg-hij',
        display_name: undefined,
        platform: undefined,
        agent_name: undefined,
        system_prompt: undefined,
        mascot_id: undefined,
        rive_colors: undefined,
      },
    });
    expect(result).toEqual({ meetUrl: 'https://meet.google.com/abc-defg-hij', platform: 'gmeet' });
  });

  it('trims whitespace from meetUrl', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      ok: true,
      meet_url: 'https://meet.google.com/abc',
      platform: 'gmeet',
    });

    await joinMeetViaBackendBot({ meetUrl: '  https://meet.google.com/abc  ' });

    expect(mockCallCoreRpc).toHaveBeenCalledWith(
      expect.objectContaining({
        params: expect.objectContaining({ meet_url: 'https://meet.google.com/abc' }),
      })
    );
  });

  it('throws on empty meetUrl', async () => {
    await expect(joinMeetViaBackendBot({ meetUrl: '  ' })).rejects.toThrow(
      'Please paste a meeting link.'
    );
    expect(mockCallCoreRpc).not.toHaveBeenCalled();
  });

  it('throws when core rejects', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ ok: false });

    await expect(joinMeetViaBackendBot({ meetUrl: 'https://meet.google.com/abc' })).rejects.toThrow(
      'Core rejected'
    );
  });

  it('forwards displayName and platform', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      ok: true,
      meet_url: 'https://zoom.us/j/123',
      platform: 'zoom',
    });

    await joinMeetViaBackendBot({
      meetUrl: 'https://zoom.us/j/123',
      displayName: 'Bot',
      platform: 'zoom',
    });

    expect(mockCallCoreRpc).toHaveBeenCalledWith(
      expect.objectContaining({
        params: expect.objectContaining({ display_name: 'Bot', platform: 'zoom' }),
      })
    );
  });

  it('forwards agentName and systemPrompt', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      ok: true,
      meet_url: 'https://meet.google.com/abc',
      platform: 'gmeet',
    });

    await joinMeetViaBackendBot({
      meetUrl: 'https://meet.google.com/abc',
      agentName: 'Aria',
      systemPrompt: 'You are a helpful meeting assistant.',
    });

    expect(mockCallCoreRpc).toHaveBeenCalledWith(
      expect.objectContaining({
        params: expect.objectContaining({
          agent_name: 'Aria',
          system_prompt: 'You are a helpful meeting assistant.',
        }),
      })
    );
  });

  it('forwards mascotId as mascot_id', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      ok: true,
      meet_url: 'https://meet.google.com/abc',
      platform: 'gmeet',
    });

    await joinMeetViaBackendBot({ meetUrl: 'https://meet.google.com/abc', mascotId: 'blue' });

    expect(mockCallCoreRpc).toHaveBeenCalledWith(
      expect.objectContaining({ params: expect.objectContaining({ mascot_id: 'blue' }) })
    );
  });

  it('trims whitespace from mascotId', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      ok: true,
      meet_url: 'https://meet.google.com/abc',
      platform: 'gmeet',
    });

    await joinMeetViaBackendBot({ meetUrl: 'https://meet.google.com/abc', mascotId: '  yellow  ' });

    expect(mockCallCoreRpc).toHaveBeenCalledWith(
      expect.objectContaining({ params: expect.objectContaining({ mascot_id: 'yellow' }) })
    );
  });

  it('sends mascot_id as undefined when mascotId is blank', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      ok: true,
      meet_url: 'https://meet.google.com/abc',
      platform: 'gmeet',
    });

    await joinMeetViaBackendBot({ meetUrl: 'https://meet.google.com/abc', mascotId: '   ' });

    expect(mockCallCoreRpc).toHaveBeenCalledWith(
      expect.objectContaining({ params: expect.objectContaining({ mascot_id: undefined }) })
    );
  });

  it('forwards riveColors as rive_colors with snake_case keys', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      ok: true,
      meet_url: 'https://meet.google.com/abc',
      platform: 'gmeet',
    });

    await joinMeetViaBackendBot({
      meetUrl: 'https://meet.google.com/abc',
      riveColors: { primaryColor: '#4A83DD', secondaryColor: '#F59E0B' },
    });

    expect(mockCallCoreRpc).toHaveBeenCalledWith(
      expect.objectContaining({
        params: expect.objectContaining({
          rive_colors: { primary_color: '#4A83DD', secondary_color: '#F59E0B' },
        }),
      })
    );
  });

  it('sends all customization fields together', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      ok: true,
      meet_url: 'https://meet.google.com/abc-defg-hij',
      platform: 'gmeet',
    });

    await joinMeetViaBackendBot({
      meetUrl: 'https://meet.google.com/abc-defg-hij',
      displayName: 'OpenHuman',
      agentName: 'Aria',
      systemPrompt: 'Be concise.',
      mascotId: 'yellow',
      riveColors: { primaryColor: '#4A83DD' },
    });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.agent_meetings_join',
      params: {
        meet_url: 'https://meet.google.com/abc-defg-hij',
        display_name: 'OpenHuman',
        platform: undefined,
        agent_name: 'Aria',
        system_prompt: 'Be concise.',
        mascot_id: 'yellow',
        rive_colors: { primary_color: '#4A83DD', secondary_color: undefined },
      },
    });
  });
});

describe('leaveBackendMeetBot', () => {
  it('calls agent_meetings_leave', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ ok: true });

    await leaveBackendMeetBot('user-requested');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.agent_meetings_leave',
      params: { reason: 'user-requested' },
    });
  });

  it('defaults reason to "requested"', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ ok: true });

    await leaveBackendMeetBot();

    expect(mockCallCoreRpc).toHaveBeenCalledWith(
      expect.objectContaining({ params: { reason: 'requested' } })
    );
  });
});

describe('sendHarnessResponse', () => {
  it('calls agent_meetings_harness_response', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ ok: true });

    await sendHarnessResponse('tool output here');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.agent_meetings_harness_response',
      params: { result: 'tool output here' },
    });
  });
});
