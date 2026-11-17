import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import MemoryCortexAnnouncement, { CORTEX_ANNOUNCEMENT_KEY } from './MemoryCortexAnnouncement';

const store = vi.hoisted(() => ({ values: new Map<string, string>(), fail: false }));

vi.mock('../../store/userScopedStorage', () => ({
  userScopedStorage: {
    getItem: vi.fn(async (key: string) => {
      if (store.fail) throw new Error('storage unavailable');
      return store.values.get(key) ?? null;
    }),
    setItem: vi.fn(async (key: string, value: string) => {
      store.values.set(key, value);
    }),
  },
}));

describe('MemoryCortexAnnouncement', () => {
  beforeEach(() => {
    store.values.clear();
    store.fail = false;
  });

  it('announces the move to CortexDB until dismissed, and stays dismissed', async () => {
    const { unmount } = renderWithProviders(<MemoryCortexAnnouncement />);
    const banner = await screen.findByTestId('memory-cortex-announcement');
    expect(banner).toHaveTextContent('Free Memory Inference on CortexDB');
    expect(banner).toHaveTextContent(
      'Basic includes 1 GB of memory storage and Pro includes 20 GB.'
    );
    expect(banner).toHaveTextContent('TinyCortex');
    expect(banner).toHaveTextContent('never used for training');
    expect(screen.getByTestId('memory-cortex-announcement-highlight')).toHaveTextContent(
      'Free migration, and memory inference is never charged.'
    );

    fireEvent.click(screen.getByTestId('memory-cortex-announcement-dismiss'));
    expect(screen.queryByTestId('memory-cortex-announcement')).not.toBeInTheDocument();
    expect(store.values.get(CORTEX_ANNOUNCEMENT_KEY)).toBe('true');

    unmount();
    renderWithProviders(<MemoryCortexAnnouncement />);
    // Give storage a chance to answer; the banner must not come back.
    await waitFor(() => expect(store.values.get(CORTEX_ANNOUNCEMENT_KEY)).toBe('true'));
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(screen.queryByTestId('memory-cortex-announcement')).not.toBeInTheDocument();
  });

  it('still shows when storage cannot be read', async () => {
    store.fail = true;
    renderWithProviders(<MemoryCortexAnnouncement />);
    expect(await screen.findByTestId('memory-cortex-announcement')).toBeInTheDocument();
  });
});
