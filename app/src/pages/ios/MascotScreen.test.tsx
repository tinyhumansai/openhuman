/**
 * MascotScreen tests — render, send message, disconnect, PTT.
 *
 * Mocks:
 *  - services/chatService: chatSend + subscribeChatEvents
 *  - services/transport/profileStore: listProfiles + deleteProfile
 *  - features/human/useHumanMascot: returns idle face
 *  - features/human/Mascot (YellowMascot): lightweight stub
 *  - react-router-dom: mock useNavigate
 *  - tauri-plugin-ptt-api: startListening, stopListening, speak, cancelSpeech,
 *    onTranscriptPartial, onError
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { MascotScreen } from './MascotScreen';

// -- module mocks ------------------------------------------------------------

const mockNavigate = vi.fn();
vi.mock('react-router-dom', async () => {
  const actual = await vi.importActual<typeof import('react-router-dom')>('react-router-dom');
  return { ...actual, useNavigate: () => mockNavigate };
});

const mockChatSend = vi.fn();
const mockUnsubscribe = vi.fn();
const mockSubscribeChatEvents = vi.fn((_listeners: unknown) => mockUnsubscribe);
vi.mock('../../services/chatService', () => ({
  chatSend: (args: unknown) => mockChatSend(args),
  subscribeChatEvents: (listeners: unknown) => mockSubscribeChatEvents(listeners),
}));

const mockListProfiles = vi.fn();
const mockDeleteProfile = vi.fn();
vi.mock('../../services/transport/profileStore', () => ({
  listProfiles: () => mockListProfiles(),
  deleteProfile: (...args: unknown[]) => mockDeleteProfile(...args),
  saveProfile: vi.fn(),
  getProfile: vi.fn(),
  listProfileIds: vi.fn(() => []),
}));

vi.mock('../../features/human/useHumanMascot', () => ({
  useHumanMascot: vi.fn(() => ({ face: 'idle', viseme: { aa: 0, E: 0, I: 0, O: 0, U: 0 } })),
}));

vi.mock('../../features/human/Mascot', () => ({
  RiveMascot: ({ face }: { face: string }) => <div data-testid="rive-mascot" data-face={face} />,
}));

// PTT plugin mock ─ intercept before any import resolution.
const mockStartListening = vi.fn();
const mockStopListening = vi.fn();
const mockSpeak = vi.fn();
const mockCancelSpeech = vi.fn();

// Listener registries so tests can fire events.
let partialListeners: Array<(text: string) => void> = [];
let pttErrorListeners: Array<(err: { code: string; message: string }) => void> = [];

const mockOnTranscriptPartial = vi.fn((cb: (text: string) => void) => {
  partialListeners.push(cb);
  const unsub = () => {
    partialListeners = partialListeners.filter(l => l !== cb);
  };
  return Promise.resolve(unsub);
});

const mockOnError = vi.fn((cb: (err: { code: string; message: string }) => void) => {
  pttErrorListeners.push(cb);
  const unsub = () => {
    pttErrorListeners = pttErrorListeners.filter(l => l !== cb);
  };
  return Promise.resolve(unsub);
});

vi.mock('tauri-plugin-ptt-api', () => ({
  startListening: () => mockStartListening(),
  stopListening: () => mockStopListening(),
  speak: (text: string, opts?: unknown) => mockSpeak(text, opts),
  cancelSpeech: () => mockCancelSpeech(),
  onTranscriptPartial: (cb: (text: string) => void) => mockOnTranscriptPartial(cb),
  onError: (cb: (err: { code: string; message: string }) => void) => mockOnError(cb),
  onTranscriptFinal: vi.fn(() => Promise.resolve(vi.fn())),
  onTtsStarted: vi.fn(() => Promise.resolve(vi.fn())),
  onTtsEnded: vi.fn(() => Promise.resolve(vi.fn())),
}));

// -- helpers -----------------------------------------------------------------

function renderMascotScreen() {
  return render(
    <MemoryRouter initialEntries={['/mascot']}>
      <MascotScreen />
    </MemoryRouter>
  );
}

function firePttPartial(text: string) {
  partialListeners.forEach(l => l(text));
}

function firePttError(code: string, message: string) {
  pttErrorListeners.forEach(l => l({ code, message }));
}

// -- setup / teardown --------------------------------------------------------

beforeEach(() => {
  mockNavigate.mockReset();
  mockChatSend.mockReset();
  mockSubscribeChatEvents.mockClear();
  mockUnsubscribe.mockReset();
  mockStartListening.mockResolvedValue(undefined);
  mockStopListening.mockResolvedValue({ text: '', isFinal: true });
  mockSpeak.mockResolvedValue(undefined);
  mockCancelSpeech.mockResolvedValue(undefined);
  mockOnTranscriptPartial.mockClear();
  mockOnError.mockClear();
  partialListeners = [];
  pttErrorListeners = [];
  mockListProfiles.mockReturnValue([{ id: 'chan1', label: 'Home desktop', kind: 'tunnel' }]);
  mockDeleteProfile.mockReset();
});

afterEach(() => {
  vi.clearAllMocks();
});

// -- tests -------------------------------------------------------------------

describe('MascotScreen', () => {
  it('renders mascot canvas and input', () => {
    renderMascotScreen();
    expect(screen.getByTestId('rive-mascot')).toBeInTheDocument();
    expect(screen.getByPlaceholderText(/type a message/i)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /send message/i })).toBeInTheDocument();
  });

  it('shows paired desktop label in header', () => {
    renderMascotScreen();
    expect(screen.getByText('Home desktop')).toBeInTheDocument();
  });

  it('shows Disconnect button', () => {
    renderMascotScreen();
    expect(screen.getByRole('button', { name: /disconnect/i })).toBeInTheDocument();
  });

  it('PTT button is present and enabled', () => {
    renderMascotScreen();
    const pttBtn = screen.getByRole('button', { name: /push to talk/i });
    expect(pttBtn).not.toBeDisabled();
  });

  it('send button is disabled when input is empty', () => {
    renderMascotScreen();
    const sendBtn = screen.getByRole('button', { name: /send message/i });
    expect(sendBtn).toBeDisabled();
  });

  it('typing a message enables send button', async () => {
    renderMascotScreen();
    const input = screen.getByPlaceholderText(/type a message/i);
    await userEvent.type(input, 'Hello mascot');
    expect(screen.getByRole('button', { name: /send message/i })).not.toBeDisabled();
  });

  it('sending a message calls chatSend with the text', async () => {
    mockChatSend.mockResolvedValueOnce(undefined);
    renderMascotScreen();

    const input = screen.getByPlaceholderText(/type a message/i);
    await userEvent.type(input, 'Hello mascot');
    await userEvent.click(screen.getByRole('button', { name: /send message/i }));

    await waitFor(() => {
      expect(mockChatSend).toHaveBeenCalledOnce();
    });
    const call = mockChatSend.mock.calls[0][0];
    expect(call.message).toBe('Hello mascot');
    expect(typeof call.threadId).toBe('string');
  });

  it('sends on Enter key press', async () => {
    mockChatSend.mockResolvedValueOnce(undefined);
    renderMascotScreen();

    const input = screen.getByPlaceholderText(/type a message/i);
    await userEvent.type(input, 'Hi{Enter}');

    await waitFor(() => {
      expect(mockChatSend).toHaveBeenCalledOnce();
    });
  });

  it('clears input after sending', async () => {
    mockChatSend.mockResolvedValueOnce(undefined);
    renderMascotScreen();

    const input = screen.getByPlaceholderText(/type a message/i);
    await userEvent.type(input, 'Hello');
    await userEvent.click(screen.getByRole('button', { name: /send message/i }));

    await waitFor(() => {
      expect((input as HTMLInputElement).value).toBe('');
    });
  });

  it('subscribes to chat events on mount', () => {
    renderMascotScreen();
    expect(mockSubscribeChatEvents).toHaveBeenCalledOnce();
  });

  it('disconnect clears profiles and navigates to /pair', async () => {
    renderMascotScreen();
    await userEvent.click(screen.getByRole('button', { name: /disconnect/i }));

    expect(mockDeleteProfile).toHaveBeenCalledWith('chan1');
    expect(mockNavigate).toHaveBeenCalledWith('/pair', { replace: true });
  });

  it('shows error message in transcript on chatSend rejection', async () => {
    mockChatSend.mockRejectedValueOnce(new Error('Network error'));
    renderMascotScreen();

    const input = screen.getByPlaceholderText(/type a message/i);
    await userEvent.type(input, 'Test message');
    await userEvent.click(screen.getByRole('button', { name: /send message/i }));

    await waitFor(() => {
      expect(screen.getByText(/failed to send/i)).toBeInTheDocument();
    });
  });

  // -- PTT tests -------------------------------------------------------------

  describe('PTT', () => {
    it('pressing PTT button calls startListening', async () => {
      renderMascotScreen();
      const pttBtn = screen.getByRole('button', { name: /push to talk/i });
      fireEvent.pointerDown(pttBtn);

      await waitFor(() => {
        expect(mockStartListening).toHaveBeenCalledOnce();
      });
    });

    it('releasing PTT calls stopListening and sends transcript as chat message', async () => {
      mockStopListening.mockResolvedValueOnce({ text: 'Hello from voice', isFinal: true });
      mockChatSend.mockResolvedValueOnce(undefined);

      renderMascotScreen();
      const pttBtn = screen.getByRole('button', { name: /push to talk/i });

      fireEvent.pointerDown(pttBtn);
      await waitFor(() => expect(mockStartListening).toHaveBeenCalledOnce());

      fireEvent.pointerUp(pttBtn);

      await waitFor(() => {
        expect(mockStopListening).toHaveBeenCalledOnce();
      });
      await waitFor(() => {
        expect(mockChatSend).toHaveBeenCalledOnce();
        const call = mockChatSend.mock.calls[0][0];
        expect(call.message).toBe('Hello from voice');
      });
    });

    it('empty transcript from stopListening does not call chatSend', async () => {
      mockStopListening.mockResolvedValueOnce({ text: '   ', isFinal: true });
      renderMascotScreen();
      const pttBtn = screen.getByRole('button', { name: /push to talk/i });

      fireEvent.pointerDown(pttBtn);
      await waitFor(() => expect(mockStartListening).toHaveBeenCalledOnce());
      fireEvent.pointerUp(pttBtn);

      await waitFor(() => expect(mockStopListening).toHaveBeenCalledOnce());
      expect(mockChatSend).not.toHaveBeenCalled();
    });

    it('PTT partial transcript updates caption above button', async () => {
      renderMascotScreen();
      const pttBtn = screen.getByRole('button', { name: /push to talk/i });
      fireEvent.pointerDown(pttBtn);

      // Fire a partial transcript event via the registered listener.
      firePttPartial('How are you');

      await waitFor(() => {
        expect(screen.getByText('How are you')).toBeInTheDocument();
      });
    });

    it('PTT error shows toast', async () => {
      renderMascotScreen();

      firePttError('permission_denied', 'Microphone access was denied.');

      await waitFor(() => {
        expect(screen.getByRole('alert')).toHaveTextContent('Microphone access was denied.');
      });
    });

    it('PTT presses cancel active TTS first', async () => {
      renderMascotScreen();
      const pttBtn = screen.getByRole('button', { name: /push to talk/i });
      fireEvent.pointerDown(pttBtn);

      await waitFor(() => {
        expect(mockCancelSpeech).toHaveBeenCalledOnce();
      });
    });

    it('startListening failure shows toast and resets button state', async () => {
      mockStartListening.mockRejectedValueOnce(new Error('No microphone'));
      renderMascotScreen();
      const pttBtn = screen.getByRole('button', { name: /push to talk/i });
      fireEvent.pointerDown(pttBtn);

      await waitFor(() => {
        expect(screen.getByRole('alert')).toHaveTextContent('No microphone');
      });
      // Button should no longer be in active (scaled) state.
      expect(pttBtn).not.toHaveClass('scale-110');
    });
  });
});
