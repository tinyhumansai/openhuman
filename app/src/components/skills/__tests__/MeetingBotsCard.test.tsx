import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import MeetingBotsCard from '../MeetingBotsCard';

const joinMock = vi.fn();

vi.mock('../../../services/meetCallService', async () => {
  const actual = await vi.importActual<typeof import('../../../services/meetCallService')>(
    '../../../services/meetCallService'
  );
  return {
    ...actual,
    joinMeetingViaMascotBot: (...args: unknown[]) => joinMock(...args),
  };
});

describe('MeetingBotsCard', () => {
  beforeEach(() => joinMock.mockReset());
  afterEach(() => cleanup());

  it('renders the banner and hides the modal by default', () => {
    render(<MeetingBotsCard />);
    expect(screen.getByTestId('meeting-bots-banner')).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('opens the modal when the banner is clicked', () => {
    render(<MeetingBotsCard />);
    fireEvent.click(screen.getByTestId('meeting-bots-banner'));
    expect(screen.getByRole('dialog')).toBeInTheDocument();
  });

  it('closes the modal on Cancel', () => {
    render(<MeetingBotsCard />);
    fireEvent.click(screen.getByTestId('meeting-bots-banner'));
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('closes the modal on Escape', () => {
    render(<MeetingBotsCard />);
    fireEvent.click(screen.getByTestId('meeting-bots-banner'));
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('submits to joinMeetingViaMascotBot and fires a success toast', async () => {
    joinMock.mockResolvedValueOnce({ success: true });
    const onToast = vi.fn();
    render(<MeetingBotsCard onToast={onToast} />);

    fireEvent.click(screen.getByTestId('meeting-bots-banner'));
    fireEvent.change(screen.getByLabelText(/meeting link/i), {
      target: { value: 'https://meet.google.com/abc-defg-hij' },
    });
    const form = screen.getByRole('dialog').querySelector('form')!;
    fireEvent.submit(form);

    await vi.waitFor(() => {
      expect(joinMock).toHaveBeenCalledWith(
        expect.objectContaining({
          platform: 'gmeet',
          meetUrl: 'https://meet.google.com/abc-defg-hij',
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

  it('surfaces a capacity-gated error inline + as an amber toast', async () => {
    joinMock.mockRejectedValueOnce({
      isCapacityGated: true,
      message: 'busy',
    });
    const onToast = vi.fn();
    render(<MeetingBotsCard onToast={onToast} />);

    fireEvent.click(screen.getByTestId('meeting-bots-banner'));
    fireEvent.change(screen.getByLabelText(/meeting link/i), {
      target: { value: 'https://meet.google.com/x' },
    });
    fireEvent.submit(screen.getByRole('dialog').querySelector('form')!);

    await vi.waitFor(() => {
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({ type: 'error', title: expect.stringMatching(/busy/i) })
      );
    });
    // Modal stays open so the user can retry; inline alert visible.
    expect(screen.getByRole('alert')).toBeInTheDocument();
  });

  it('surfaces a non-capacity error', async () => {
    joinMock.mockRejectedValueOnce({ isCapacityGated: false, message: 'Bad URL' });
    const onToast = vi.fn();
    render(<MeetingBotsCard onToast={onToast} />);

    fireEvent.click(screen.getByTestId('meeting-bots-banner'));
    fireEvent.change(screen.getByLabelText(/meeting link/i), {
      target: { value: 'https://meet.google.com/x' },
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
    render(<MeetingBotsCard />);
    fireEvent.click(screen.getByTestId('meeting-bots-banner'));
    // Pick Zoom (coming soon)
    fireEvent.click(screen.getByRole('button', { name: /Zoom/ }));
    const submit = screen.getByRole('button', { name: /coming soon/i });
    expect(submit).toBeDisabled();
  });
});
