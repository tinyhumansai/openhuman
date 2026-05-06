import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { MemorySyncStatus } from '../../services/memorySyncService';
import { MemorySyncConnections } from './MemorySyncConnections';

const mockStatusList = vi.fn();

vi.mock('../../services/memorySyncService', async () => {
  const actual = await vi.importActual<typeof import('../../services/memorySyncService')>(
    '../../services/memorySyncService'
  );
  return { ...actual, memorySyncStatusList: (...args: unknown[]) => mockStatusList(...args) };
});

function makeStatus(overrides: Partial<MemorySyncStatus> = {}): MemorySyncStatus {
  return {
    provider: 'gmail',
    chunks_synced: 0,
    chunks_pending: 0,
    batch_total: 0,
    batch_processed: 0,
    last_chunk_at_ms: null,
    freshness: 'idle',
    ...overrides,
  };
}

describe('<MemorySyncConnections />', () => {
  beforeEach(() => {
    mockStatusList.mockReset();
  });

  it('renders a card per provider from the RPC', async () => {
    mockStatusList.mockResolvedValueOnce([
      makeStatus({ provider: 'gmail', chunks_synced: 42, freshness: 'active' }),
      makeStatus({ provider: 'discord', chunks_synced: 7, freshness: 'recent' }),
    ]);
    render(<MemorySyncConnections />);
    await waitFor(() => {
      expect(screen.getByTestId('memory-sync-card-gmail')).toBeTruthy();
      expect(screen.getByTestId('memory-sync-card-discord')).toBeTruthy();
    });
    expect(screen.getByText('Gmail')).toBeTruthy();
    expect(screen.getByText('Discord')).toBeTruthy();
  });

  it('shows chunk count + freshness label', async () => {
    mockStatusList.mockResolvedValueOnce([
      makeStatus({ provider: 'notion', chunks_synced: 1234, freshness: 'recent' }),
    ]);
    render(<MemorySyncConnections />);
    await waitFor(() => {
      expect(screen.getByTestId('memory-sync-chunks-notion').textContent).toContain('1,234');
      expect(screen.getByTestId('memory-sync-freshness-notion').textContent).toBe('Recent');
    });
  });

  it('renders the empty state when the RPC returns []', async () => {
    mockStatusList.mockResolvedValueOnce([]);
    render(<MemorySyncConnections />);
    await waitFor(() => {
      expect(screen.getByText(/No content has been synced/)).toBeTruthy();
    });
  });

  it('renders the failure state when the RPC throws', async () => {
    mockStatusList.mockRejectedValueOnce(new Error('rpc unavailable'));
    render(<MemorySyncConnections />);
    await waitFor(() => {
      expect(screen.getByText(/Failed to load sync status/)).toBeTruthy();
      expect(screen.getByText(/rpc unavailable/)).toBeTruthy();
    });
  });
});
