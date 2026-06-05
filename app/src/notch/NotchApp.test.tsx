import { render, screen, waitFor } from '@testing-library/react';
import { act } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { connectCoreSocket } from '../services/coreSocket';
import NotchApp from './NotchApp';

// Key-passthrough i18n that honours the inline fallback the component passes
// (`t('notch.listening', 'Listening…')`).
vi.mock('../lib/i18n/I18nContext', () => ({
  useT: () => ({ t: (key: string, fallback?: string) => fallback ?? key }),
}));

vi.mock('../services/coreSocket', () => ({ connectCoreSocket: vi.fn() }));

// Minimal Socket.IO stand-in: records handlers so a test can replay events.
class MockSocket {
  static last: MockSocket | null = null;
  handlers = new Map<string, (payload: unknown) => void>();
  connected = false;
  id = 'notch-test-socket';
  connect = vi.fn(() => {
    this.connected = true;
    return this;
  });
  disconnect = vi.fn(() => {
    this.connected = false;
    return this;
  });
  on = vi.fn((event: string, handler: (payload: unknown) => void) => {
    this.handlers.set(event, handler);
    return this;
  });
  fire(event: string, payload: unknown) {
    const handler = this.handlers.get(event);
    if (!handler) throw new Error(`no handler registered for ${event}`);
    act(() => handler(payload));
  }
  constructor() {
    MockSocket.last = this;
  }
}

describe('NotchApp', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    MockSocket.last = null;
    (window as { __OPENHUMAN_NOTCH_CORE_URL__?: string }).__OPENHUMAN_NOTCH_CORE_URL__ =
      'http://127.0.0.1:9999';
    vi.mocked(connectCoreSocket).mockImplementation(
      async () => new MockSocket() as unknown as Awaited<ReturnType<typeof connectCoreSocket>>
    );
  });

  afterEach(() => {
    delete (window as { __OPENHUMAN_NOTCH_CORE_URL__?: string }).__OPENHUMAN_NOTCH_CORE_URL__;
  });

  const renderAndConnect = async () => {
    render(<NotchApp />);
    // Idle baseline pill.
    expect(screen.getByText('Ready')).toBeInTheDocument();
    // Wait for the async connect to register its socket handlers.
    await waitFor(() => expect(MockSocket.last).not.toBeNull());
    await waitFor(() =>
      expect(MockSocket.last?.on).toHaveBeenCalledWith('overlay:attention', expect.any(Function))
    );
    return MockSocket.last as MockSocket;
  };

  it('connects using the preloaded core URL and shows the idle pill', async () => {
    const socket = await renderAndConnect();
    expect(connectCoreSocket).toHaveBeenCalledTimes(1);
    expect(socket.connect).toHaveBeenCalled();
  });

  it('renders Listening on a dictation press', async () => {
    const socket = await renderAndConnect();
    socket.fire('dictation:toggle', { type: 'pressed' });
    expect(await screen.findByText('Listening…')).toBeInTheDocument();
  });

  it('renders the transcript text on dictation:transcription', async () => {
    const socket = await renderAndConnect();
    socket.fire('dictation:transcription', { text: 'play some music' });
    expect(await screen.findByText('play some music')).toBeInTheDocument();
  });

  it('maps companion:state_changed to a mode', async () => {
    const socket = await renderAndConnect();
    socket.fire('companion:state_changed', { state: 'thinking' });
    expect(await screen.findByText('Processing…')).toBeInTheDocument();
  });

  it('renders an overlay:attention message', async () => {
    const socket = await renderAndConnect();
    socket.fire('overlay:attention', { message: 'Opening Music', ttl_ms: 5000 });
    expect(await screen.findByText('Opening Music')).toBeInTheDocument();
  });

  it('handles speaking, released and idle transitions without throwing', async () => {
    const socket = await renderAndConnect();
    socket.fire('companion:state_changed', { state: 'speaking' });
    expect(await screen.findByText('Speaking…')).toBeInTheDocument();
    // Released schedules a dismiss; idle drives an immediate dismiss — both
    // exercise the scheduleDismiss branches.
    socket.fire('dictation:toggle', { type: 'released' });
    socket.fire('companion:state_changed', { state: 'idle' });
  });

  it('connects via the notch:core-url event when no URL was preloaded', async () => {
    delete (window as { __OPENHUMAN_NOTCH_CORE_URL__?: string }).__OPENHUMAN_NOTCH_CORE_URL__;
    render(<NotchApp />);
    expect(screen.getByText('Ready')).toBeInTheDocument();
    expect(connectCoreSocket).not.toHaveBeenCalled();

    act(() =>
      window.dispatchEvent(
        new CustomEvent('notch:core-url', { detail: { url: 'http://127.0.0.1:8888' } })
      )
    );
    await waitFor(() => expect(connectCoreSocket).toHaveBeenCalledTimes(1));
  });
});
