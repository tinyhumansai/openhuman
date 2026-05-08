import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { closeMeetCall, joinMeetCall } from '../../services/meetCallService';
import IntelligenceCallsTab from './IntelligenceCallsTab';

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }));

vi.mock('../../services/meetCallService', () => ({
  joinMeetCall: vi.fn(),
  closeMeetCall: vi.fn(),
}));

const VALID_URL = 'https://meet.google.com/abc-defg-hij';

describe('IntelligenceCallsTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('renders form with URL + display name inputs and a disabled join button', () => {
    render(<IntelligenceCallsTab />);

    expect(screen.getByRole('heading', { name: /Join a Google Meet call/i })).toBeInTheDocument();
    const urlInput = screen.getByPlaceholderText(/meet\.google\.com/i);
    expect(urlInput).toBeInTheDocument();
    // Display name has a default value, so the join button is enabled only
    // once the URL field is also non-empty. With an empty URL it stays
    // disabled.
    expect(screen.getByRole('button', { name: /Join call/i })).toBeDisabled();
  });

  it('calls joinMeetCall on submit and adds the result to the active-call list', async () => {
    vi.mocked(joinMeetCall).mockResolvedValueOnce({
      requestId: 'req-1',
      meetUrl: VALID_URL,
      displayName: 'OpenHuman Agent',
      windowLabel: 'meet-call-req-1',
    });

    const onToast = vi.fn();
    render(<IntelligenceCallsTab onToast={onToast} />);

    fireEvent.change(screen.getByPlaceholderText(/meet\.google\.com/i), {
      target: { value: VALID_URL },
    });
    fireEvent.click(screen.getByRole('button', { name: /Join call/i }));

    await waitFor(() => expect(joinMeetCall).toHaveBeenCalledTimes(1));
    expect(joinMeetCall).toHaveBeenCalledWith({
      meetUrl: VALID_URL,
      displayName: 'OpenHuman Agent',
    });

    // Active call appears with a Leave button.
    await screen.findByText('OpenHuman Agent');
    expect(screen.getByRole('button', { name: /Leave/i })).toBeInTheDocument();
    expect(onToast).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'success', title: 'Joining call' })
    );
  });

  it('renders the rejection reason in the form when joinMeetCall throws', async () => {
    vi.mocked(joinMeetCall).mockRejectedValueOnce(new Error('Core rejected the request'));
    const onToast = vi.fn();

    render(<IntelligenceCallsTab onToast={onToast} />);
    fireEvent.change(screen.getByPlaceholderText(/meet\.google\.com/i), {
      target: { value: VALID_URL },
    });
    fireEvent.click(screen.getByRole('button', { name: /Join call/i }));

    await screen.findByRole('alert');
    expect(screen.getByRole('alert')).toHaveTextContent(/Core rejected the request/i);
    expect(onToast).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'error', title: 'Could not start call' })
    );
  });

  it('falls back to a generic error message for non-Error rejections', async () => {
    // joinMeetCall throws a non-Error value (e.g. a raw string) — the
    // component should still surface a sane message instead of crashing.
    vi.mocked(joinMeetCall).mockRejectedValueOnce('boom');

    render(<IntelligenceCallsTab />);
    fireEvent.change(screen.getByPlaceholderText(/meet\.google\.com/i), {
      target: { value: VALID_URL },
    });
    fireEvent.click(screen.getByRole('button', { name: /Join call/i }));

    await screen.findByRole('alert');
    expect(screen.getByRole('alert')).toHaveTextContent(/Failed to start Meet call/i);
  });

  it('removes the call from the list when the user clicks Leave', async () => {
    vi.mocked(joinMeetCall).mockResolvedValueOnce({
      requestId: 'req-2',
      meetUrl: VALID_URL,
      displayName: 'OpenHuman Agent',
      windowLabel: 'meet-call-req-2',
    });
    vi.mocked(closeMeetCall).mockResolvedValueOnce(true);

    render(<IntelligenceCallsTab />);
    fireEvent.change(screen.getByPlaceholderText(/meet\.google\.com/i), {
      target: { value: VALID_URL },
    });
    fireEvent.click(screen.getByRole('button', { name: /Join call/i }));

    const leaveBtn = await screen.findByRole('button', { name: /Leave/i });
    fireEvent.click(leaveBtn);

    await waitFor(() => expect(closeMeetCall).toHaveBeenCalledWith('req-2'));
    await waitFor(() =>
      expect(screen.queryByRole('button', { name: /Leave/i })).not.toBeInTheDocument()
    );
  });

  it('keeps the row when closeMeetCall returns false (window stayed open)', async () => {
    vi.mocked(joinMeetCall).mockResolvedValueOnce({
      requestId: 'req-3',
      meetUrl: VALID_URL,
      displayName: 'OpenHuman Agent',
      windowLabel: 'meet-call-req-3',
    });
    vi.mocked(closeMeetCall).mockResolvedValueOnce(false);

    render(<IntelligenceCallsTab />);
    fireEvent.change(screen.getByPlaceholderText(/meet\.google\.com/i), {
      target: { value: VALID_URL },
    });
    fireEvent.click(screen.getByRole('button', { name: /Join call/i }));

    const leaveBtn = await screen.findByRole('button', { name: /Leave/i });
    fireEvent.click(leaveBtn);

    await waitFor(() => expect(closeMeetCall).toHaveBeenCalledWith('req-3'));
    // Row stays so the user can retry; the meet-call:closed event listener
    // would still drop it later if the shell ends up tearing the window
    // down on its own.
    expect(screen.getByRole('button', { name: /Leave/i })).toBeInTheDocument();
  });
});
