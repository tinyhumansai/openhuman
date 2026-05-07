import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { WhatsAppMemorySection } from './WhatsAppMemorySection';

const mockWhatsappListChats = vi.fn();

vi.mock('../../utils/tauriCommands/memory', () => ({
  whatsappListChats: (...args: unknown[]) => mockWhatsappListChats(...args),
}));

function makeChat(overrides: Record<string, unknown> = {}) {
  return {
    chat_id: 'chat-1',
    display_name: 'Test Chat',
    is_group: false,
    account_id: 'acc-1',
    last_message_ts: 1_700_000_000,
    message_count: 5,
    updated_at: 1_700_000_000,
    ...overrides,
  };
}

describe('<WhatsAppMemorySection />', () => {
  beforeEach(() => {
    mockWhatsappListChats.mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('renders nothing when load returns an empty array', async () => {
    mockWhatsappListChats.mockResolvedValueOnce([]);
    const { container } = render(<WhatsAppMemorySection pollIntervalMs={0} />);
    await waitFor(() => expect(mockWhatsappListChats).toHaveBeenCalled());
    expect(container.firstChild).toBeNull();
  });

  it('stays hidden when load throws (error is swallowed silently)', async () => {
    mockWhatsappListChats.mockRejectedValueOnce(new Error('scanner not ready'));
    const { container } = render(<WhatsAppMemorySection pollIntervalMs={0} />);
    await waitFor(() => expect(mockWhatsappListChats).toHaveBeenCalled());
    expect(container.firstChild).toBeNull();
  });

  it('calls whatsappListChats with limit:200', async () => {
    mockWhatsappListChats.mockResolvedValueOnce([]);
    render(<WhatsAppMemorySection pollIntervalMs={0} />);
    await waitFor(() => expect(mockWhatsappListChats).toHaveBeenCalledWith({ limit: 200 }));
  });

  it('renders section and plural "chats" when multiple chats load', async () => {
    mockWhatsappListChats.mockResolvedValueOnce([
      makeChat({ chat_id: 'c1' }),
      makeChat({ chat_id: 'c2' }),
    ]);
    render(<WhatsAppMemorySection pollIntervalMs={0} />);
    await waitFor(() => screen.getByTestId('whatsapp-memory-section'));
    expect(screen.getByText(/2 chats synced/)).toBeTruthy();
  });

  it('renders singular "chat" for exactly 1 chat', async () => {
    mockWhatsappListChats.mockResolvedValueOnce([makeChat()]);
    render(<WhatsAppMemorySection pollIntervalMs={0} />);
    await waitFor(() => screen.getByTestId('whatsapp-memory-section'));
    const el = screen.getByText(/chat synced/);
    expect(el.textContent).toMatch(/^1 chat synced/);
  });

  it('shows "just now" when delta < 60s', async () => {
    const updatedAt = 1_700_000_000;
    vi.spyOn(Date, 'now').mockReturnValue((updatedAt + 30) * 1000);
    mockWhatsappListChats.mockResolvedValueOnce([makeChat({ updated_at: updatedAt })]);
    render(<WhatsAppMemorySection pollIntervalMs={0} />);
    await waitFor(() => screen.getByTestId('whatsapp-memory-section'));
    expect(screen.getByText(/just now/)).toBeTruthy();
  });

  it('shows "Xm ago" for 60-3599s delta', async () => {
    const updatedAt = 1_700_000_000;
    vi.spyOn(Date, 'now').mockReturnValue((updatedAt + 120) * 1000);
    mockWhatsappListChats.mockResolvedValueOnce([makeChat({ updated_at: updatedAt })]);
    render(<WhatsAppMemorySection pollIntervalMs={0} />);
    await waitFor(() => screen.getByTestId('whatsapp-memory-section'));
    expect(screen.getByText(/2m ago/)).toBeTruthy();
  });

  it('shows "Xh ago" for 3600-86399s delta', async () => {
    const updatedAt = 1_700_000_000;
    vi.spyOn(Date, 'now').mockReturnValue((updatedAt + 7200) * 1000);
    mockWhatsappListChats.mockResolvedValueOnce([makeChat({ updated_at: updatedAt })]);
    render(<WhatsAppMemorySection pollIntervalMs={0} />);
    await waitFor(() => screen.getByTestId('whatsapp-memory-section'));
    expect(screen.getByText(/2h ago/)).toBeTruthy();
  });

  it('shows "Xd ago" for delta >= 86400s', async () => {
    const updatedAt = 1_700_000_000;
    vi.spyOn(Date, 'now').mockReturnValue((updatedAt + 86400 * 3) * 1000);
    mockWhatsappListChats.mockResolvedValueOnce([makeChat({ updated_at: updatedAt })]);
    render(<WhatsAppMemorySection pollIntervalMs={0} />);
    await waitFor(() => screen.getByTestId('whatsapp-memory-section'));
    expect(screen.getByText(/3d ago/)).toBeTruthy();
  });

  it('omits timestamp when all chats have updated_at = 0', async () => {
    mockWhatsappListChats.mockResolvedValueOnce([makeChat({ updated_at: 0 })]);
    render(<WhatsAppMemorySection pollIntervalMs={0} />);
    await waitFor(() => screen.getByTestId('whatsapp-memory-section'));
    const el = screen.getByText(/chat synced/);
    expect(el.textContent).not.toMatch(/ago|just now/);
  });

  it('handleSync: clicking Sync reloads data and updates count', async () => {
    mockWhatsappListChats
      .mockResolvedValueOnce([makeChat()])
      .mockResolvedValueOnce([makeChat({ chat_id: 'c1' }), makeChat({ chat_id: 'c2' })]);
    render(<WhatsAppMemorySection pollIntervalMs={0} />);
    await waitFor(() => screen.getByTestId('whatsapp-memory-section'));

    fireEvent.click(screen.getByRole('button'));
    await waitFor(() => expect(mockWhatsappListChats).toHaveBeenCalledTimes(2));
    await waitFor(() => screen.getByText(/2 chats synced/));
  });

  it('handleSync: button shows "Syncing…" and is disabled while load is in flight', async () => {
    mockWhatsappListChats.mockResolvedValueOnce([makeChat()]);
    render(<WhatsAppMemorySection pollIntervalMs={0} />);
    await waitFor(() => screen.getByTestId('whatsapp-memory-section'));

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    let resolveSync!: (v: any) => void;
    mockWhatsappListChats.mockReturnValueOnce(
      new Promise(r => {
        resolveSync = r;
      })
    );
    fireEvent.click(screen.getByRole('button'));

    await waitFor(() => screen.getByText('Syncing…'));
    expect(screen.getByRole('button')).toBeDisabled();

    await act(async () => {
      resolveSync([makeChat()]);
    });
    await waitFor(() => screen.getByText('Sync'));
  });
});
