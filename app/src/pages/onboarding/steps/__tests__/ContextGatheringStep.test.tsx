import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import ContextGatheringStep from '../ContextGatheringStep';

const callCoreRpc = vi.hoisted(() => vi.fn());
vi.mock('../../../../services/coreRpcClient', () => ({ callCoreRpc }));

describe('ContextGatheringStep', () => {
  beforeEach(() => {
    callCoreRpc.mockReset();
  });

  it('no-Gmail branch: auto-navigates without any RPC', async () => {
    vi.useFakeTimers();
    const onNext = vi.fn().mockResolvedValue(undefined);
    renderWithProviders(<ContextGatheringStep connectedSources={['notion']} onNext={onNext} />);

    await act(async () => {
      vi.advanceTimersByTime(850);
    });
    expect(onNext).toHaveBeenCalled();
    expect(callCoreRpc).not.toHaveBeenCalled();
    vi.useRealTimers();
  });

  it('shows building animation and auto-starts pipeline on mount', async () => {
    // Keep the pipeline pending so we can assert the animation state
    let resolveGmail!: (v: unknown) => void;
    callCoreRpc.mockImplementation(async (req: { method: string }) => {
      if (req.method === 'openhuman.tools_composio_execute') {
        return new Promise(res => {
          resolveGmail = res;
        });
      }
      throw new Error(`unexpected RPC ${req.method}`);
    });

    renderWithProviders(
      <ContextGatheringStep
        connectedSources={['composio:gmail']}
        onNext={() => Promise.resolve()}
      />
    );

    expect(screen.getByText(/building your profile/i)).toBeInTheDocument();
    // Stage labels from the old UI should not be visible
    expect(screen.queryByText(/Processing your Gmail/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/Working on your LinkedIn/i)).not.toBeInTheDocument();
    // Pipeline started automatically — no button click needed
    expect(callCoreRpc).toHaveBeenCalled();

    // Unblock so no timers leak
    await act(async () => {
      resolveGmail({ successful: true, data: { messages: [] } });
    });
  });

  it('runs the full Gmail -> Apify scrape -> save pipeline and auto-navigates', async () => {
    const onNext = vi.fn().mockResolvedValue(undefined);
    callCoreRpc.mockImplementation(async (req: { method: string; params: unknown }) => {
      if (req.method === 'openhuman.tools_composio_execute') {
        return {
          successful: true,
          data: {
            messages: [
              { messageText: 'Visit https://www.linkedin.com/comm/in/jane-doe?foo=bar to view.' },
            ],
          },
        };
      }
      if (req.method === 'openhuman.tools_apify_linkedin_scrape') {
        return {
          data: { name: 'Jane Doe', headline: 'Founder at Acme' },
          markdown: '# Jane Doe\n\nFounder at Acme. Based in Berlin.',
        };
      }
      if (req.method === 'openhuman.learning_save_profile') {
        return { path: '/tmp/PROFILE.md', bytes: 256 };
      }
      throw new Error(`unexpected RPC ${req.method}`);
    });

    renderWithProviders(
      <ContextGatheringStep connectedSources={['composio:gmail']} onNext={onNext} />
    );

    await waitFor(() => expect(onNext).toHaveBeenCalled(), { timeout: 5000 });

    const calls = callCoreRpc.mock.calls.map((c: Array<{ method: string }>) => c[0].method);
    expect(calls).toEqual([
      'openhuman.tools_composio_execute',
      'openhuman.tools_apify_linkedin_scrape',
      'openhuman.learning_save_profile',
    ]);

    const scrapeCall = callCoreRpc.mock.calls.find(
      (c: Array<{ method: string }>) => c[0].method === 'openhuman.tools_apify_linkedin_scrape'
    );
    expect((scrapeCall![0].params as { profile_url: string }).profile_url).toBe(
      'https://www.linkedin.com/in/jane-doe'
    );

    const saveCall = callCoreRpc.mock.calls.find(
      (c: Array<{ method: string }>) => c[0].method === 'openhuman.learning_save_profile'
    );
    expect(saveCall![0].params.summarize).toBe(true);
    expect(saveCall![0].params.markdown).toContain('Founder at Acme');
  });

  it('skips downstream stages when Gmail finds no LinkedIn URL and auto-navigates', async () => {
    const onNext = vi.fn().mockResolvedValue(undefined);
    callCoreRpc.mockResolvedValueOnce({
      successful: true,
      data: { messages: [{ messageText: 'Hello, no linkedin link here.' }] },
    });

    renderWithProviders(
      <ContextGatheringStep connectedSources={['composio:gmail']} onNext={onNext} />
    );

    await waitFor(() => expect(callCoreRpc).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(onNext).toHaveBeenCalled(), { timeout: 5000 });
  });

  describe('background continuation link', () => {
    afterEach(() => {
      vi.useRealTimers();
    });

    it('does not show background link before 10s threshold', async () => {
      vi.useFakeTimers();
      let resolveGmail!: (v: unknown) => void;
      callCoreRpc.mockImplementation(
        () =>
          new Promise(res => {
            resolveGmail = res;
          })
      );

      renderWithProviders(
        <ContextGatheringStep connectedSources={['composio:gmail']} onNext={vi.fn()} />
      );

      await act(async () => {
        vi.advanceTimersByTime(9_999);
      });
      expect(screen.queryByText(/keep building in background/i)).not.toBeInTheDocument();

      // Cleanup
      await act(async () => {
        resolveGmail({ successful: true, data: { messages: [] } });
      });
    });

    it('shows background link after 10s with fade-in animation', async () => {
      vi.useFakeTimers();
      let resolveGmail!: (v: unknown) => void;
      callCoreRpc.mockImplementation(
        () =>
          new Promise(res => {
            resolveGmail = res;
          })
      );

      renderWithProviders(
        <ContextGatheringStep connectedSources={['composio:gmail']} onNext={vi.fn()} />
      );

      await act(async () => {
        vi.advanceTimersByTime(10_000);
      });
      const link = screen.getByText(/keep building in background/i);
      expect(link).toBeInTheDocument();
      expect(link.className).toContain('animate-fade-in');

      await act(async () => {
        resolveGmail({ successful: true, data: { messages: [] } });
      });
    });

    it('clicking background link calls onNext to complete onboarding', async () => {
      vi.useFakeTimers();
      let resolveGmail!: (v: unknown) => void;
      callCoreRpc.mockImplementation(
        () =>
          new Promise(res => {
            resolveGmail = res;
          })
      );
      const onNext = vi.fn().mockResolvedValue(undefined);

      renderWithProviders(
        <ContextGatheringStep connectedSources={['composio:gmail']} onNext={onNext} />
      );

      await act(async () => {
        vi.advanceTimersByTime(10_000);
      });
      fireEvent.click(screen.getByText(/keep building in background/i));
      expect(onNext).toHaveBeenCalledTimes(1);

      await act(async () => {
        resolveGmail({ successful: true, data: { messages: [] } });
      });
    });

    it('does not show background link if pipeline finishes within 10s', async () => {
      vi.useFakeTimers();
      callCoreRpc.mockResolvedValue({ successful: true, data: { messages: [] } });

      renderWithProviders(
        <ContextGatheringStep connectedSources={['composio:gmail']} onNext={vi.fn()} />
      );

      // Let pipeline resolve (microtasks)
      await act(async () => {
        await Promise.resolve();
      });

      // Advance past 10s — link should still not appear since finished is true
      await act(async () => {
        vi.advanceTimersByTime(11_000);
      });
      expect(screen.queryByText(/keep building in background/i)).not.toBeInTheDocument();
    });

    it('pipeline saves profile even after user clicks background link and component unmounts', async () => {
      vi.useFakeTimers();
      let resolveScrape!: (v: unknown) => void;
      let resolveSave!: (v: unknown) => void;

      callCoreRpc.mockImplementation(async (req: { method: string }) => {
        if (req.method === 'openhuman.tools_composio_execute') {
          return {
            successful: true,
            data: { messages: [{ messageText: 'https://www.linkedin.com/in/test-user' }] },
          };
        }
        if (req.method === 'openhuman.tools_apify_linkedin_scrape') {
          return new Promise(res => {
            resolveScrape = res;
          });
        }
        if (req.method === 'openhuman.learning_save_profile') {
          return new Promise(res => {
            resolveSave = res;
          });
        }
        throw new Error(`unexpected RPC ${req.method}`);
      });

      const onNext = vi.fn().mockResolvedValue(undefined);
      const { unmount } = renderWithProviders(
        <ContextGatheringStep connectedSources={['composio:gmail']} onNext={onNext} />
      );

      // Wait for Gmail stage to complete and scrape to start
      await act(async () => {
        await Promise.resolve();
        await Promise.resolve();
      });

      // Show background link after 10s
      await act(async () => {
        vi.advanceTimersByTime(10_000);
      });

      // User clicks background link → onNext called, then unmount
      fireEvent.click(screen.getByText(/keep building in background/i));
      expect(onNext).toHaveBeenCalled();
      unmount();

      // Resolve remaining pipeline stages after unmount
      await act(async () => {
        resolveScrape({ data: {}, markdown: '# Test User' });
        await Promise.resolve();
        await Promise.resolve();
      });

      await act(async () => {
        resolveSave({ path: '/tmp/PROFILE.md', bytes: 128 });
        await Promise.resolve();
      });

      // Verify save_profile was called — pipeline continued after unmount
      const saveCalls = callCoreRpc.mock.calls.filter(
        (c: Array<{ method: string }>) => c[0].method === 'openhuman.learning_save_profile'
      );
      expect(saveCalls.length).toBe(1);
    });
  });

  it('shows friendly error message when learning_save_profile rejects', async () => {
    const onNext = vi.fn().mockResolvedValue(undefined);
    callCoreRpc.mockImplementation(async (req: { method: string; params: unknown }) => {
      if (req.method === 'openhuman.tools_composio_execute') {
        return {
          successful: true,
          data: { messages: [{ messageText: 'https://www.linkedin.com/in/jane-doe' }] },
        };
      }
      if (req.method === 'openhuman.tools_apify_linkedin_scrape') {
        return { data: { name: 'Jane Doe' }, markdown: '# Jane Doe\n\nFounder at Acme.' };
      }
      if (req.method === 'openhuman.learning_save_profile') {
        throw new Error('disk full');
      }
      throw new Error(`unexpected RPC ${req.method}`);
    });

    renderWithProviders(
      <ContextGatheringStep connectedSources={['composio:gmail']} onNext={onNext} />
    );

    await waitFor(() => {
      expect(
        screen.getByText(/we couldn't build your full profile right now/i)
      ).toBeInTheDocument();
    });

    expect(screen.getByRole('button', { name: 'Continue' })).toBeInTheDocument();
    expect(screen.queryByText('disk full')).not.toBeInTheDocument();

    // fireEvent not needed — onNext is available via the button but user can also
    // just verify the friendly message is shown
  });
});
