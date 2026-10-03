import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { ConversationsSettings } from '../../services/api/memoryApi';
import { renderWithProviders } from '../../test/test-utils';
import MemoryConversationsTab from './MemoryConversationsTab';

const hoisted = vi.hoisted(() => ({ get: vi.fn(), set: vi.fn() }));

vi.mock('../../services/api/memoryApi', async importOriginal => ({
  ...(await importOriginal<typeof import('../../services/api/memoryApi')>()),
  memoryConversationsGet: (...a: unknown[]) => hoisted.get(...a),
  memoryConversationsSet: (...a: unknown[]) => hoisted.set(...a),
}));

const SETTINGS: ConversationsSettings = {
  enabled: true,
  batch_turns: 4,
  idle_secs: 120,
  recent: [{ thread_id: 'thread-1', turns: 6, stored_at: '2026-10-01T10:00:00Z' }],
};

beforeEach(() => {
  hoisted.get.mockReset().mockResolvedValue(SETTINGS);
  hoisted.set.mockReset();
});

describe('MemoryConversationsTab', () => {
  it('shows the settings and the recently stored threads', async () => {
    renderWithProviders(<MemoryConversationsTab />);
    expect(await screen.findByTestId('memory-conversation-thread-1')).toHaveTextContent('6 turns');
    expect(screen.getByLabelText('Turns per batch')).toHaveValue(4);
    expect(screen.getByLabelText('Save when idle')).toHaveValue(120);
  });

  it('turns automatic storage off', async () => {
    hoisted.set.mockResolvedValue({ ...SETTINGS, enabled: false });
    renderWithProviders(<MemoryConversationsTab />);
    fireEvent.click(await screen.findByTestId('memory-conversations-enabled'));
    await waitFor(() => expect(hoisted.set).toHaveBeenCalledWith({ enabled: false }));
    await waitFor(() => expect(screen.getByLabelText('Turns per batch')).toBeDisabled());
  });

  it('saves a new batch size on commit and ignores an invalid one', async () => {
    hoisted.set.mockResolvedValue({ ...SETTINGS, batch_turns: 8 });
    renderWithProviders(<MemoryConversationsTab />);
    const batch = await screen.findByLabelText('Turns per batch');

    fireEvent.change(batch, { target: { value: '0' } });
    fireEvent.blur(batch);
    expect(hoisted.set).not.toHaveBeenCalled();
    expect(batch).toHaveValue(4);

    fireEvent.change(batch, { target: { value: '8' } });
    fireEvent.blur(batch);
    await waitFor(() => expect(hoisted.set).toHaveBeenCalledWith({ batch_turns: 8 }));
  });

  it('shows the empty recent list', async () => {
    hoisted.get.mockResolvedValue({ ...SETTINGS, recent: [] });
    renderWithProviders(<MemoryConversationsTab />);
    expect(await screen.findByTestId('memory-conversations-empty')).toBeInTheDocument();
  });

  it('shows a load error', async () => {
    hoisted.get.mockRejectedValue(new Error('MEMORY_OFF'));
    renderWithProviders(<MemoryConversationsTab />);
    expect(await screen.findByTestId('memory-conversations-error')).toHaveTextContent('MEMORY_OFF');
  });
});
