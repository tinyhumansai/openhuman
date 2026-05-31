import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { MeetCallRecord } from '../../../services/meetCallService';
import { renderWithProviders } from '../../../test/test-utils';
import MeetingBotsCard, { MeetingBotsModal } from '../MeetingBotsCard';

const joinMock = vi.fn();
const listMock = vi.fn();

vi.mock('../../../services/meetCallService', async () => {
  const actual = await vi.importActual<typeof import('../../../services/meetCallService')>(
    '../../../services/meetCallService'
  );
  return {
    ...actual,
    // Flow A: the modal submit calls joinMeetCall (CEF webview), not the
    // Flow B backend joinMeetingViaMascotBot. Switched in the
    // mascot-meet-flowA revival commits — kept the mock variable name
    // `joinMock` to keep the diff focused on the call site swap.
    joinMeetCall: (...args: unknown[]) => joinMock(...args),
    listMeetCalls: (...args: unknown[]) => listMock(...args),
  };
});

describe('MeetingBotsCard', () => {
  beforeEach(() => {
    joinMock.mockReset();
    listMock.mockReset();
    // Default: resolve with empty list so modal renders without flashing errors.
    listMock.mockResolvedValue([]);
  });
  afterEach(() => cleanup());

  it('renders the banner and hides the modal by default', () => {
    renderWithProviders(<MeetingBotsCard />);
    expect(screen.getByTestId('meeting-bots-banner')).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('opens the modal when the banner is clicked', () => {
    renderWithProviders(<MeetingBotsCard />);
    fireEvent.click(screen.getByTestId('meeting-bots-banner'));
    expect(screen.getByRole('dialog')).toBeInTheDocument();
  });

  it('closes the modal on Cancel', () => {
    renderWithProviders(<MeetingBotsCard />);
    fireEvent.click(screen.getByTestId('meeting-bots-banner'));
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('closes the modal on Escape', () => {
    renderWithProviders(<MeetingBotsCard />);
    fireEvent.click(screen.getByTestId('meeting-bots-banner'));
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('submits to joinMeetCall and fires a success toast', async () => {
    joinMock.mockResolvedValueOnce({ requestId: 'req-1' });
    const onToast = vi.fn();
    renderWithProviders(<MeetingBotsCard onToast={onToast} />);

    fireEvent.click(screen.getByTestId('meeting-bots-banner'));
    fireEvent.change(screen.getByLabelText(/meeting link/i), {
      target: { value: 'https://meet.google.com/abc-defg-hij' },
    });
    // Owner display name is now required — the wake-word gate refuses
    // every caption when this is empty (privacy lock), so the submit
    // button stays disabled and the test would hang on form submit
    // without typing a value here.
    fireEvent.change(screen.getByLabelText(/your name in the call/i), {
      target: { value: 'Alice' },
    });
    const form = screen.getByRole('dialog').querySelector('form')!;
    fireEvent.submit(form);

    // Flow A's joinMeetCall takes { meetUrl, displayName, ownerDisplayName }.
    // Assert on the owner name (the new privacy-lock contract) and meetUrl;
    // the bot displayName is a UI-supplied default and not contract-load-
    // bearing for this assertion.
    await vi.waitFor(() => {
      expect(joinMock).toHaveBeenCalledWith(
        expect.objectContaining({
          meetUrl: 'https://meet.google.com/abc-defg-hij',
          ownerDisplayName: 'Alice',
        })
      );
    });
    await vi.waitFor(() => {
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({ type: 'success', title: expect.stringMatching(/joining/i) })
      );
    });
    // Modal closes on success
    await vi.waitFor(() => {
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    });
  });

  // Flow A's joinMeetCall has no capacity-gated concept — any throw maps
  // to the single "could not start" toast + inline alert with the error
  // message. Two error cases collapsed into one in the Flow A model.
  it('surfaces a join error inline + as an error toast', async () => {
    joinMock.mockRejectedValueOnce(new Error('Bad URL'));
    const onToast = vi.fn();
    renderWithProviders(<MeetingBotsCard onToast={onToast} />);

    fireEvent.click(screen.getByTestId('meeting-bots-banner'));
    fireEvent.change(screen.getByLabelText(/meeting link/i), {
      target: { value: 'https://meet.google.com/x' },
    });
    fireEvent.change(screen.getByLabelText(/your name in the call/i), {
      target: { value: 'Alice' },
    });
    fireEvent.submit(screen.getByRole('dialog').querySelector('form')!);

    await vi.waitFor(() => {
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({ type: 'error', title: expect.stringMatching(/not start/i) })
      );
    });
    expect(screen.getByRole('alert')).toHaveTextContent('Bad URL');
  });

  it('disables the submit when the active platform is coming-soon', () => {
    renderWithProviders(<MeetingBotsCard />);
    fireEvent.click(screen.getByTestId('meeting-bots-banner'));
    // Pick Zoom (coming soon)
    fireEvent.click(screen.getByRole('button', { name: /Zoom/ }));
    const submit = screen.getByRole('button', { name: /coming soon/i });
    expect(submit).toBeDisabled();
  });

  // Issue #2945: users were being asked to retype "your name in the call"
  // every meeting. When the Persona display name (Settings → Persona) is
  // set, the owner-name field pre-fills from it so the user can submit
  // without retyping.
  it('pre-fills the owner display name from the Persona slice', () => {
    const { store } = renderWithProviders(<MeetingBotsCard />, {
      preloadedState: { persona: { displayName: 'Hemanth', description: '' } },
    });
    fireEvent.click(screen.getByTestId('meeting-bots-banner'));
    const ownerInput = screen.getByLabelText(/your name in the call/i) as HTMLInputElement;
    expect(ownerInput.value).toBe('Hemanth');
    // Sanity: the slice is wired so the assertion above isn't a no-op.
    expect(store.getState().persona.displayName).toBe('Hemanth');
  });

  it('leaves the owner display name empty when no Persona name is set', () => {
    // Default preloadedState — persona slice initial state is
    // { displayName: '', description: '' } (see personaSlice.ts).
    renderWithProviders(<MeetingBotsCard />);
    fireEvent.click(screen.getByTestId('meeting-bots-banner'));
    const ownerInput = screen.getByLabelText(/your name in the call/i) as HTMLInputElement;
    expect(ownerInput.value).toBe('');
  });

  // Dirty-flag contract: once the user has typed into the field, a
  // subsequent Persona update from the slice must NOT overwrite their
  // input. The pre-edit case is covered by the "pre-fills" test above.
  it('does not overwrite user-typed owner name when Persona slice updates later', () => {
    const { store } = renderWithProviders(<MeetingBotsCard />, {
      preloadedState: { persona: { displayName: 'Hemanth', description: '' } },
    });
    fireEvent.click(screen.getByTestId('meeting-bots-banner'));
    const ownerInput = screen.getByLabelText(/your name in the call/i) as HTMLInputElement;
    // Sanity: pre-fill landed.
    expect(ownerInput.value).toBe('Hemanth');
    // User types over it.
    fireEvent.change(ownerInput, { target: { value: 'Alice' } });
    expect(ownerInput.value).toBe('Alice');
    // Persona name changes underneath (e.g. user edits Settings in another window).
    store.dispatch({ type: 'persona/setPersonaDisplayName', payload: 'Nova' });
    // User's typed value wins — does NOT flip to 'Nova'.
    expect(ownerInput.value).toBe('Alice');
  });
});

// ── RecentCallsSection / RecentCallRow tests ──────────────────────────────────
// These exercise the listMeetCalls integration inside MeetingBotsModal:
// loading state, empty state, error state, and populated list.

function makeCallRecord(overrides: Partial<MeetCallRecord> = {}): MeetCallRecord {
  return {
    request_id: 'req-1',
    meet_url: 'https://meet.google.com/abc-defg-hij',
    bot_display_name: 'OpenHuman',
    owner_display_name: 'Alice',
    started_at_ms: Date.now() - 5 * 60 * 1000, // 5 minutes ago
    ended_at_ms: Date.now() - 4 * 60 * 1000,
    listened_seconds: 30,
    spoken_seconds: 30,
    turn_count: 3,
    ...overrides,
  };
}

describe('MeetingBotsModal — recent calls section', () => {
  afterEach(() => cleanup());

  it('shows a loading hint while listMeetCalls is pending', () => {
    // Never resolves during this test — simulates a slow fetch.
    listMock.mockReturnValue(new Promise(() => {}));

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    expect(screen.getByText(/loading…/i)).toBeInTheDocument();
  });

  it('shows an empty-state message when listMeetCalls returns an empty array', async () => {
    listMock.mockResolvedValueOnce([]);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText(/no previous calls yet/i)).toBeInTheDocument();
    });
  });

  it('renders a row for each returned call record', async () => {
    const records = [
      makeCallRecord({ request_id: 'req-1', meet_url: 'https://meet.google.com/aaa-bbbb-ccc', turn_count: 2 }),
      makeCallRecord({ request_id: 'req-2', meet_url: 'https://meet.google.com/ddd-eeee-fff', turn_count: 5 }),
    ];
    listMock.mockResolvedValueOnce(records);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText('aaa-bbbb-ccc')).toBeInTheDocument();
      expect(screen.getByText('ddd-eeee-fff')).toBeInTheDocument();
    });
    // turn counts shown in the row detail line
    expect(screen.getByText(/2 turns/i)).toBeInTheDocument();
    expect(screen.getByText(/5 turns/i)).toBeInTheDocument();
  });

  it('shows the count badge when there is at least one record', async () => {
    listMock.mockResolvedValueOnce([makeCallRecord()]);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      // The "(1)" count badge next to the "Recent calls" heading.
      expect(screen.getByText('(1)')).toBeInTheDocument();
    });
  });

  it('shows an error hint and an empty list when listMeetCalls rejects', async () => {
    listMock.mockRejectedValueOnce(new Error('Network timeout'));

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText(/network timeout/i)).toBeInTheDocument();
    });
    // After the error the rows state falls back to [] — no loading hint.
    expect(screen.queryByText(/loading…/i)).not.toBeInTheDocument();
  });

  it('strips the https://meet.google.com/ prefix and shows only the meeting code', async () => {
    listMock.mockResolvedValueOnce([
      makeCallRecord({ meet_url: 'https://meet.google.com/xyz-1234-abc' }),
    ]);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText('xyz-1234-abc')).toBeInTheDocument();
    });
    // Full URL should NOT be visible — only the code portion.
    expect(screen.queryByText('https://meet.google.com/xyz-1234-abc')).not.toBeInTheDocument();
  });

  it('shows duration as combined spoken + listened seconds', async () => {
    listMock.mockResolvedValueOnce([
      makeCallRecord({ spoken_seconds: 40, listened_seconds: 20 }),
    ]);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText(/60s on call/i)).toBeInTheDocument();
    });
  });

  it('shows a relative timestamp for recent calls', async () => {
    // started 5 minutes ago
    listMock.mockResolvedValueOnce([
      makeCallRecord({ started_at_ms: Date.now() - 5 * 60 * 1000 }),
    ]);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText(/\dm ago/)).toBeInTheDocument();
    });
  });

  it('shows "—" for a zero started_at_ms timestamp', async () => {
    listMock.mockResolvedValueOnce([makeCallRecord({ started_at_ms: 0 })]);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText('—')).toBeInTheDocument();
    });
  });

  // ── Extra coverage for RecentCallRow / formatRelativeTime branches ──────────

  it('shows singular "turn" (not "turns") when turn_count is 1', async () => {
    listMock.mockResolvedValueOnce([makeCallRecord({ turn_count: 1 })]);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText(/1 turn$/)).toBeInTheDocument();
    });
    expect(screen.queryByText(/1 turns/)).not.toBeInTheDocument();
  });

  it('falls back to the raw URL when it cannot be parsed', async () => {
    listMock.mockResolvedValueOnce([makeCallRecord({ meet_url: 'not-a-valid-url' })]);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText('not-a-valid-url')).toBeInTheDocument();
    });
  });

  it('shows hours-ago label for a timestamp a few hours old', async () => {
    listMock.mockResolvedValueOnce([
      makeCallRecord({ started_at_ms: Date.now() - 3 * 60 * 60 * 1000 }),
    ]);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText(/3h ago/)).toBeInTheDocument();
    });
  });

  it('shows "yesterday" for a timestamp ~24 hours ago', async () => {
    listMock.mockResolvedValueOnce([
      makeCallRecord({ started_at_ms: Date.now() - 25 * 60 * 60 * 1000 }),
    ]);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText('yesterday')).toBeInTheDocument();
    });
  });

  it('shows Nd-ago label for a timestamp a few days old (< 7)', async () => {
    listMock.mockResolvedValueOnce([
      makeCallRecord({ started_at_ms: Date.now() - 3 * 24 * 60 * 60 * 1000 }),
    ]);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText(/3d ago/)).toBeInTheDocument();
    });
  });

  it('shows a locale date string for a timestamp older than 7 days', async () => {
    listMock.mockResolvedValueOnce([
      makeCallRecord({ started_at_ms: Date.now() - 10 * 24 * 60 * 60 * 1000 }),
    ]);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      // toLocaleDateString returns "Month Day" — just check it's not a relative label.
      const timestamp = screen.queryByText(/ago|yesterday|\dm|\dh/);
      expect(timestamp).not.toBeInTheDocument();
    });
  });
});
