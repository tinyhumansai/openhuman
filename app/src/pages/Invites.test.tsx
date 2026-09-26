import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { useUser } from '../hooks/useUser';
import { inviteApi } from '../services/api/inviteApi';
import type { InviteCode } from '../types/invite';
import Invites from './Invites';

const { mockAuth } = vi.hoisted(() => ({ mockAuth: { credential: 'session' as string } }));

vi.mock('../hooks/useUser', () => ({ useUser: vi.fn() }));
vi.mock('../providers/CoreStateProvider', () => ({
  useCoreState: () => ({
    snapshot: {
      auth: { isAuthenticated: true, credential: mockAuth.credential },
      sessionToken: null,
    },
  }),
}));
vi.mock('../services/api/inviteApi', () => ({
  inviteApi: { getMyInviteCodes: vi.fn(), redeemInviteCode: vi.fn() },
}));

const mockUseUser = vi.mocked(useUser);
const getMyInviteCodes = vi.mocked(inviteApi.getMyInviteCodes);
const writeText = vi.fn().mockResolvedValue(undefined);

const INVITE: InviteCode = {
  _id: 'invite-1',
  code: 'TEST-CODE',
  owner: 'owner-1',
  type: 'USER',
  maxUses: 1,
  currentUses: 0,
  usageHistory: [],
  isActive: true,
  createdAt: '2026-01-02T00:00:00Z',
};

beforeEach(() => {
  vi.clearAllMocks();
  mockAuth.credential = 'session';
  writeText.mockResolvedValue(undefined);
  Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
  mockUseUser.mockReturnValue({
    user: { referral: { invitedBy: 'inviter-1' } },
    isLoading: false,
    error: null,
    refetch: vi.fn().mockResolvedValue(undefined),
  } as unknown as ReturnType<typeof useUser>);
  getMyInviteCodes.mockResolvedValue([INVITE]);
});

describe('Invites clipboard feedback', () => {
  test('preserves copy success feedback, its 2000ms duration, and analytics behavior', async () => {
    try {
      render(<Invites />);
      await screen.findByText(INVITE.code);
      const copyButton = screen.getByRole('button', { name: 'Copy' });
      expect(copyButton).not.toHaveAttribute('data-analytics-id');

      vi.useFakeTimers();
      await act(async () => {
        fireEvent.click(copyButton);
        await Promise.resolve();
      });
      expect(writeText).toHaveBeenCalledWith(INVITE.code);
      expect(copyButton.querySelector('svg.text-sage-500')).toBeInTheDocument();

      act(() => {
        vi.advanceTimersByTime(1999);
      });
      expect(copyButton.querySelector('svg.text-sage-500')).toBeInTheDocument();
      act(() => {
        vi.advanceTimersByTime(1);
      });
      expect(copyButton.querySelector('svg.text-sage-500')).not.toBeInTheDocument();
      expect(copyButton).toHaveAccessibleName('Copy');
    } finally {
      vi.useRealTimers();
    }
  });

  test('preserves the Copy error label when clipboard writing fails', async () => {
    writeText.mockRejectedValueOnce(new Error('clipboard unavailable'));
    render(<Invites />);
    await screen.findByText(INVITE.code);
    const copyButton = screen.getByRole('button', { name: 'Copy' });

    fireEvent.click(copyButton);

    await waitFor(() => expect(writeText).toHaveBeenCalledWith(INVITE.code));
    expect(copyButton).toHaveAccessibleName('Copy');
    expect(copyButton.querySelector('svg.text-sage-500')).not.toBeInTheDocument();
  });
});

describe('Invites without a hosted account', () => {
  test('explains the offline local profile instead of fetching invite codes', async () => {
    mockAuth.credential = 'local';
    render(<Invites />);
    expect(await screen.findByText(/Local login does not earn rewards/)).toBeInTheDocument();
    expect(getMyInviteCodes).not.toHaveBeenCalled();
  });
});
