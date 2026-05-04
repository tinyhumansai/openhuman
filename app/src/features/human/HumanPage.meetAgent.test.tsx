import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

// ─── Tests ─────────────────────────────────────────────────────────────────

import HumanPage from './HumanPage';

// ─── Mocks ─────────────────────────────────────────────────────────────────

const mockMeetAgentJoin = vi.fn().mockResolvedValue(undefined);
const mockMeetAgentLeave = vi.fn().mockResolvedValue(undefined);
let capturedMeetAgentHandler: ((e: unknown) => void) | null = null;

vi.mock('../../services/meetAgent', () => ({
  meetAgentJoin: (...args: unknown[]) => mockMeetAgentJoin(...args),
  meetAgentLeave: (...args: unknown[]) => mockMeetAgentLeave(...args),
  subscribeMeetAgentEvents: vi.fn((handler: (e: unknown) => void) => {
    capturedMeetAgentHandler = handler;
    return () => {
      capturedMeetAgentHandler = null;
    };
  }),
}));

// Mock the mascot hook so we don't drag audio / viseme deps.
vi.mock('./useHumanMascot', () => ({ useHumanMascot: () => ({ face: 'idle', viseme: null }) }));

// Mock Mascot so we don't need SVG imports.
vi.mock('./Mascot', () => ({ Ghosty: () => <div data-testid="ghosty" /> }));

// Mock Conversations sidebar.
vi.mock('../../pages/Conversations', () => ({
  default: () => <div data-testid="conversations" />,
}));

// Mock config — default to non-production so the panel renders.
const mockAppEnvironment = vi.hoisted(() => ({ value: 'staging' as string }));
vi.mock('../../utils/config', () => ({
  get APP_ENVIRONMENT() {
    return mockAppEnvironment.value;
  },
}));

function renderPage() {
  return render(<HumanPage />);
}

beforeEach(() => {
  localStorage.clear();
  capturedMeetAgentHandler = null;
  mockMeetAgentJoin.mockResolvedValue(undefined);
  mockMeetAgentLeave.mockResolvedValue(undefined);
  mockAppEnvironment.value = 'staging';
});

describe('MeetAgentPanel staging gate', () => {
  it('renders the panel in staging environment', () => {
    mockAppEnvironment.value = 'staging';
    renderPage();
    expect(screen.getByTestId('meet-agent-join')).toBeTruthy();
  });

  it('renders the panel in development environment', () => {
    mockAppEnvironment.value = 'development';
    renderPage();
    expect(screen.getByTestId('meet-agent-join')).toBeTruthy();
  });

  it('hides the panel in production environment', () => {
    mockAppEnvironment.value = 'production';
    renderPage();
    expect(screen.queryByTestId('meet-agent-join')).toBeNull();
  });
});

describe('MeetAgentPanel interactions', () => {
  it('Join button calls meetAgentJoin with typed inputs', async () => {
    renderPage();

    fireEvent.change(screen.getByTestId('meet-agent-account-id'), {
      target: { value: 'acct-123' },
    });
    fireEvent.change(screen.getByTestId('meet-agent-meeting-url'), {
      target: { value: 'https://meet.google.com/abc-defg-hij' },
    });
    fireEvent.click(screen.getByTestId('meet-agent-join'));

    await waitFor(() => {
      expect(mockMeetAgentJoin).toHaveBeenCalledWith({
        accountId: 'acct-123',
        meetingUrl: 'https://meet.google.com/abc-defg-hij',
      });
    });
  });

  it('Leave button calls meetAgentLeave', async () => {
    renderPage();

    fireEvent.change(screen.getByTestId('meet-agent-account-id'), {
      target: { value: 'acct-123' },
    });
    fireEvent.click(screen.getByTestId('meet-agent-leave'));

    await waitFor(() => {
      expect(mockMeetAgentLeave).toHaveBeenCalledWith({ accountId: 'acct-123' });
    });
  });

  it('status line updates on meet_agent_joined event', async () => {
    renderPage();

    // Wait for subscription to register.
    await waitFor(() => expect(capturedMeetAgentHandler).not.toBeNull());

    capturedMeetAgentHandler!({
      kind: 'meet_agent_joined',
      accountId: 'acct-123',
      code: 'abc-defg-hij',
      joinedAt: new Date('2025-01-01T12:34:00').getTime(),
    });

    await waitFor(() => {
      const status = screen.getByTestId('meet-agent-status').textContent ?? '';
      expect(status).toMatch(/joined\s+abc-defg-hij/);
    });
  });

  it('status line updates on meet_agent_left event', async () => {
    renderPage();
    await waitFor(() => expect(capturedMeetAgentHandler).not.toBeNull());

    capturedMeetAgentHandler!({
      kind: 'meet_agent_left',
      accountId: 'acct-123',
      reason: 'leave-button-gone',
    });

    await waitFor(() => {
      const status = screen.getByTestId('meet-agent-status').textContent ?? '';
      expect(status).toContain('leave-button-gone');
    });
  });

  it('status line updates on meet_agent_failed event', async () => {
    renderPage();
    await waitFor(() => expect(capturedMeetAgentHandler).not.toBeNull());

    capturedMeetAgentHandler!({
      kind: 'meet_agent_failed',
      accountId: 'acct-123',
      reason: 'timeout',
    });

    await waitFor(() => {
      const status = screen.getByTestId('meet-agent-status').textContent ?? '';
      expect(status).toContain('timeout');
    });
  });
});
