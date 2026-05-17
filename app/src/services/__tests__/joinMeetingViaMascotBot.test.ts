import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  joinMeetingViaMascotBot,
  type MascotJoinMeetingError,
  SERVER_OVERLOADED_MESSAGE,
} from '../meetCallService';

const postMock = vi.fn();

vi.mock('../apiClient', () => ({ apiClient: { post: (...args: unknown[]) => postMock(...args) } }));

describe('joinMeetingViaMascotBot', () => {
  beforeEach(() => postMock.mockReset());

  it('rejects an empty meet URL with isCapacityGated=false', async () => {
    await expect(
      joinMeetingViaMascotBot({ platform: 'gmeet', meetUrl: '   ' })
    ).rejects.toMatchObject({ isCapacityGated: false, message: expect.stringMatching(/link/i) });
    expect(postMock).not.toHaveBeenCalled();
  });

  it('POSTs the trimmed payload on the happy path', async () => {
    postMock.mockResolvedValueOnce({ success: true });
    const res = await joinMeetingViaMascotBot({
      platform: 'gmeet',
      meetUrl: '  https://meet.google.com/abc-defg-hij  ',
      displayName: '  OpenHuman  ',
    });
    expect(res).toEqual({ success: true });
    expect(postMock).toHaveBeenCalledWith('/mascots/join-meeting', {
      platform: 'gmeet',
      meetUrl: 'https://meet.google.com/abc-defg-hij',
      displayName: 'OpenHuman',
    });
  });

  it('drops empty displayName to undefined', async () => {
    postMock.mockResolvedValueOnce({ success: true });
    await joinMeetingViaMascotBot({
      platform: 'gmeet',
      meetUrl: 'https://meet.google.com/x',
      displayName: '   ',
    });
    expect(postMock).toHaveBeenCalledWith('/mascots/join-meeting', {
      platform: 'gmeet',
      meetUrl: 'https://meet.google.com/x',
      displayName: undefined,
    });
  });

  it('flags SERVER_OVERLOADED responses with isCapacityGated=true', async () => {
    postMock.mockRejectedValueOnce({ success: false, error: SERVER_OVERLOADED_MESSAGE });
    let caught: MascotJoinMeetingError | undefined;
    try {
      await joinMeetingViaMascotBot({ platform: 'gmeet', meetUrl: 'https://meet.google.com/abc' });
    } catch (e) {
      caught = e as MascotJoinMeetingError;
    }
    expect(caught?.isCapacityGated).toBe(true);
    expect(caught?.message).toBe(SERVER_OVERLOADED_MESSAGE);
  });

  it('passes through other apiClient errors with isCapacityGated=false', async () => {
    postMock.mockRejectedValueOnce({ success: false, error: 'Bad Request' });
    await expect(
      joinMeetingViaMascotBot({ platform: 'zoom', meetUrl: 'https://zoom.us/j/1' })
    ).rejects.toMatchObject({ isCapacityGated: false, message: 'Bad Request' });
  });

  it('wraps non-ApiError throwables', async () => {
    postMock.mockRejectedValueOnce(new Error('network down'));
    await expect(
      joinMeetingViaMascotBot({ platform: 'gmeet', meetUrl: 'https://meet.google.com/x' })
    ).rejects.toMatchObject({ isCapacityGated: false, message: 'network down' });
  });
});
