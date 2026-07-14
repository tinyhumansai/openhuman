import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { createRef } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { setBackendMeetError } from '../../../store/backendMeetSlice';
import { renderWithProviders } from '../../../test/test-utils';
import { MeetComposer } from '../MeetComposer';

type Toast = { type: 'success' | 'error' | 'info'; title: string; message?: string };

const joinMock = vi.fn();
const listMock = vi.fn();

vi.mock('../../../services/meetCallService', async () => {
  const actual = await vi.importActual<typeof import('../../../services/meetCallService')>(
    '../../../services/meetCallService'
  );
  return {
    ...actual,
    joinMeetViaBackendBot: (...args: unknown[]) => joinMock(...args),
    listMeetCalls: (...args: unknown[]) => listMock(...args),
  };
});

const mockConnectionByToolkit = vi.fn(
  () => new Map<string, { id: string; toolkit: string; status: string; accountEmail?: string }>()
);

vi.mock('../../../lib/composio/hooks', () => ({
  useComposioIntegrations: () => ({
    toolkits: [],
    connectionByToolkit: mockConnectionByToolkit(),
    connectionsByToolkit: new Map(),
    catalogByToolkit: new Map(),
    loading: false,
    error: null,
    refresh: vi.fn(),
  }),
}));

function renderComposer(props: { onToast?: ReturnType<typeof vi.fn> } = {}) {
  const hasSubmittedRef = createRef<boolean>() as React.MutableRefObject<boolean>;
  hasSubmittedRef.current = false;
  // Cast the vitest mock to the expected callback type — vi.fn() returns a
  // broad Mock type that TypeScript doesn't automatically narrow to the
  // specific callback signature the component expects.
  const onToast = props.onToast as unknown as ((toast: Toast) => void) | undefined;
  const result = renderWithProviders(
    <MeetComposer hasSubmittedRef={hasSubmittedRef} onToast={onToast} />
  );
  return { ...result, hasSubmittedRef };
}

describe('MeetComposer', () => {
  beforeEach(() => {
    joinMock.mockReset();
    listMock.mockReset();
    listMock.mockResolvedValue([]);
    mockConnectionByToolkit.mockReturnValue(new Map());
  });
  afterEach(() => cleanup());

  it('renders the join form with meeting link and name inputs', () => {
    renderComposer();
    expect(screen.getByLabelText(/meeting link/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/your name in this meeting/i)).toBeInTheDocument();
  });

  it('defaults to Google Meet and shows the correct submit label', () => {
    renderComposer();
    // Submit button label starts with "Send to Google Meet"
    expect(screen.getByRole('button', { name: /send to google meet/i })).toBeInTheDocument();
  });

  it('updates submit label and URL placeholder when switching platforms', async () => {
    renderComposer();

    // Switch to Zoom
    const zoomChip = screen.getByRole('radio', { name: /zoom/i });
    fireEvent.click(zoomChip);

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /send to zoom/i })).toBeInTheDocument();
    });

    const urlInput = screen.getByLabelText(/meeting link/i);
    expect(urlInput).toHaveAttribute('placeholder', 'zoom.us/j/...');
  });

  it('updates submit label and URL placeholder for Webex', async () => {
    renderComposer();

    const webexChip = screen.getByRole('radio', { name: /webex/i });
    fireEvent.click(webexChip);

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /send to webex/i })).toBeInTheDocument();
    });

    const urlInput = screen.getByLabelText(/meeting link/i);
    expect(urlInput).toHaveAttribute('placeholder', 'webex.com/meet/...');
  });

  it('prefills the name field from a connected Composio account', async () => {
    mockConnectionByToolkit.mockReturnValue(
      new Map([
        [
          'googlemeet',
          {
            id: 'c1',
            toolkit: 'googlemeet',
            status: 'ACTIVE',
            accountEmail: 'alice.smith@gmail.com',
          },
        ],
      ])
    );

    renderComposer();

    await waitFor(() => {
      expect(screen.getByLabelText(/your name in this meeting/i)).toHaveValue('Alice Smith');
    });
  });

  it('re-derives name from platform-specific account when platform changes (untouched)', async () => {
    mockConnectionByToolkit.mockReturnValue(
      new Map([
        [
          'googlemeet',
          {
            id: 'c1',
            toolkit: 'googlemeet',
            status: 'ACTIVE',
            accountEmail: 'alice.gmeet@company.com',
          },
        ],
        [
          'zoom',
          { id: 'c2', toolkit: 'zoom', status: 'ACTIVE', accountEmail: 'alice.zoom@company.com' },
        ],
      ])
    );

    renderComposer();

    // Initially fills from gmeet
    await waitFor(() => {
      expect(screen.getByLabelText(/your name in this meeting/i)).toHaveValue('Alice Gmeet');
    });

    // Switch to zoom — should re-derive from zoom account
    fireEvent.click(screen.getByRole('radio', { name: /zoom/i }));

    await waitFor(() => {
      expect(screen.getByLabelText(/your name in this meeting/i)).toHaveValue('Alice Zoom');
    });
  });

  it('never overwrites a manually typed name when platform changes', async () => {
    renderComposer();

    const nameInput = screen.getByLabelText(/your name in this meeting/i);

    // User types manually
    fireEvent.change(nameInput, { target: { value: 'Custom Name' } });

    // Switch platform
    fireEvent.click(screen.getByRole('radio', { name: /zoom/i }));

    // Manually typed value must not be overwritten
    expect(nameInput).toHaveValue('Custom Name');
  });

  it('submits with the SELECTED platform (not hardcoded gmeet)', async () => {
    joinMock.mockResolvedValueOnce({ meetUrl: 'https://zoom.us/j/123', platform: 'zoom' });

    renderComposer();

    // Switch to Zoom
    fireEvent.click(screen.getByRole('radio', { name: /zoom/i }));

    fireEvent.change(screen.getByLabelText(/meeting link/i), {
      target: { value: 'https://zoom.us/j/123' },
    });
    fireEvent.change(screen.getByLabelText(/your name in this meeting/i), {
      target: { value: 'Alice' },
    });
    const form = document.querySelector('form')!;
    fireEvent.submit(form);

    await waitFor(() => {
      expect(joinMock).toHaveBeenCalledWith(
        expect.objectContaining({
          meetUrl: 'https://zoom.us/j/123',
          platform: 'zoom',
          respondToParticipant: 'Alice',
          listenOnly: false,
        })
      );
    });
  });

  it('submits with gmeet by default (no platform switch)', async () => {
    joinMock.mockResolvedValueOnce({
      meetUrl: 'https://meet.google.com/abc-defg-hij',
      platform: 'gmeet',
    });

    renderComposer();

    fireEvent.change(screen.getByLabelText(/meeting link/i), {
      target: { value: 'https://meet.google.com/abc-defg-hij' },
    });
    fireEvent.change(screen.getByLabelText(/your name in this meeting/i), {
      target: { value: 'Alice' },
    });
    const form = document.querySelector('form')!;
    fireEvent.submit(form);

    await waitFor(() => {
      expect(joinMock).toHaveBeenCalledWith(expect.objectContaining({ platform: 'gmeet' }));
    });
  });

  it('shows an inline error when the backend returns an error via Redux', async () => {
    const onToast = vi.fn();
    const { hasSubmittedRef, store } = renderComposer({ onToast });

    // Simulate submission having been started
    hasSubmittedRef.current = true;

    // Backend fires an error event
    store.dispatch(setBackendMeetError({ error: 'Bot failed to join.' }));

    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent('Bot failed to join.');
    });

    expect(onToast).toHaveBeenCalledWith(expect.objectContaining({ type: 'error' }));
  });

  it('shows the capacity-gate message when the server is overloaded', async () => {
    const onToast = vi.fn();
    const { hasSubmittedRef, store } = renderComposer({ onToast });

    hasSubmittedRef.current = true;

    store.dispatch(
      setBackendMeetError({
        error: 'Mascot streaming capacity is exhausted. Please try again later.',
      })
    );

    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent(/heavy load/i);
    });
  });

  it('shows an inline error when joinMeetViaBackendBot throws synchronously', async () => {
    joinMock.mockRejectedValueOnce(new Error('Network error.'));
    const onToast = vi.fn();
    const { hasSubmittedRef } = renderComposer({ onToast });

    fireEvent.change(screen.getByLabelText(/meeting link/i), {
      target: { value: 'https://meet.google.com/abc-defg-hij' },
    });
    fireEvent.change(screen.getByLabelText(/your name in this meeting/i), {
      target: { value: 'Alice' },
    });
    const form = document.querySelector('form')!;
    fireEvent.submit(form);

    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent('Network error.');
    });

    // hasSubmittedRef should be reset so a second attempt works
    expect(hasSubmittedRef.current).toBe(false);
    expect(onToast).toHaveBeenCalledWith(expect.objectContaining({ type: 'error' }));
  });

  it('disables submit when meetUrl or name is empty', () => {
    renderComposer();

    const submitBtn = screen.getByRole('button', { name: /send to google meet/i });

    // Both empty — disabled
    expect(submitBtn).toBeDisabled();

    // Fill URL only
    fireEvent.change(screen.getByLabelText(/meeting link/i), {
      target: { value: 'https://meet.google.com/abc' },
    });
    expect(submitBtn).toBeDisabled();

    // Fill name too — enabled
    fireEvent.change(screen.getByLabelText(/your name in this meeting/i), {
      target: { value: 'Alice' },
    });
    expect(submitBtn).not.toBeDisabled();
  });
});
