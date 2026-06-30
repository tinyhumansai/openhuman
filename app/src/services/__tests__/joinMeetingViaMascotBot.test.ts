import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  isCapacityGateMessage,
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

  it('flags the real backend SERVER_OVERLOADED wording and shows the tailored copy (#4151)', async () => {
    // The VERBATIM backend message (backend src/utils/paidPlan.ts) — NOT the
    // frontend's friendly constant. Mocking the real wire string is what makes
    // this test catch the prior exact-equality mismatch that leaked the generic
    // "…try again later." text to users.
    const BACKEND_CAPACITY_MESSAGE =
      'Mascot streaming capacity is exhausted. Please try again later.';
    postMock.mockRejectedValueOnce({ success: false, error: BACKEND_CAPACITY_MESSAGE });
    let caught: MascotJoinMeetingError | undefined;
    try {
      await joinMeetingViaMascotBot({ platform: 'gmeet', meetUrl: 'https://meet.google.com/abc' });
    } catch (e) {
      caught = e as MascotJoinMeetingError;
    }
    expect(caught?.isCapacityGated).toBe(true);
    // The tailored, actionable copy is surfaced — not the raw backend string.
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

describe('isCapacityGateMessage', () => {
  it('matches the real backend capacity wording (and case/wording drift)', () => {
    expect(
      isCapacityGateMessage('Mascot streaming capacity is exhausted. Please try again later.')
    ).toBe(true);
    expect(isCapacityGateMessage('MASCOT STREAMING CAPACITY IS EXHAUSTED.')).toBe(true);
    expect(isCapacityGateMessage('Streaming capacity reached — retry soon')).toBe(true);
  });

  it('does not match unrelated errors or empty input', () => {
    expect(isCapacityGateMessage('Bad Request')).toBe(false);
    expect(isCapacityGateMessage('network down')).toBe(false);
    expect(isCapacityGateMessage('')).toBe(false);
    expect(isCapacityGateMessage(null)).toBe(false);
    expect(isCapacityGateMessage(undefined)).toBe(false);
  });
});
