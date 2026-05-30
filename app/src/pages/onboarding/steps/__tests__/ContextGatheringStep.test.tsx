import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import ContextGatheringStep from '../ContextGatheringStep';

const callCoreRpc = vi.hoisted(() => vi.fn());
const getCoreRpcUrl = vi.hoisted(() => vi.fn(async () => 'http://127.0.0.1:7788/rpc'));
const testCoreRpcConnection = vi.hoisted(() =>
  vi.fn(async () => ({ ok: true, status: 200 }) as Response)
);
const captureException = vi.hoisted(() => vi.fn());
vi.mock('../../../../services/coreRpcClient', () => ({
  callCoreRpc,
  getCoreRpcUrl,
  testCoreRpcConnection,
}));
vi.mock('@sentry/react', () => ({ captureException }));

describe('ContextGatheringStep', () => {
  beforeEach(() => {
    callCoreRpc.mockReset();
    getCoreRpcUrl.mockClear();
    testCoreRpcConnection.mockClear();
    testCoreRpcConnection.mockResolvedValue({ ok: true, status: 200 } as Response);
    captureException.mockReset();
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

  it('runs Gmail -> save pipeline with Apify disabled and auto-navigates', async () => {
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
    expect(calls).toEqual(['openhuman.tools_composio_execute', 'openhuman.learning_save_profile']);
    // Apify scrape must not be called — it is disabled during profile build.
    expect(calls).not.toContain('openhuman.tools_apify_linkedin_scrape');

    const saveCall = callCoreRpc.mock.calls.find(
      (c: Array<{ method: string }>) => c[0].method === 'openhuman.learning_save_profile'
    );
    expect(saveCall![0].params.summarize).toBe(true);
    expect(saveCall![0].params.markdown).toContain('https://www.linkedin.com/in/jane-doe');
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

  describe('non-blocking continuation', () => {
    afterEach(() => {
      vi.useRealTimers();
    });

    it('lets users continue to chat immediately while integration work is slow', async () => {
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

      expect(screen.getByRole('button', { name: /continue to chat/i })).toBeInTheDocument();

      await act(async () => {
        resolveGmail({ successful: true, data: { messages: [] } });
      });
    });

    it('clicking continue calls onNext before the pipeline finishes', async () => {
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

      fireEvent.click(screen.getByRole('button', { name: /continue to chat/i }));
      expect(onNext).toHaveBeenCalledTimes(1);

      await act(async () => {
        resolveGmail({ successful: true, data: { messages: [] } });
      });
    });

    it('hides the manual continue button if the pipeline finishes quickly', async () => {
      callCoreRpc.mockResolvedValue({ successful: true, data: { messages: [] } });

      renderWithProviders(
        <ContextGatheringStep connectedSources={['composio:gmail']} onNext={vi.fn()} />
      );

      // Let pipeline resolve (microtasks)
      await act(async () => {
        await Promise.resolve();
      });

      expect(screen.queryByRole('button', { name: /continue to chat/i })).not.toBeInTheDocument();
    });

    it('pipeline saves profile even after user continues and component unmounts', async () => {
      let resolveSave!: (v: unknown) => void;

      callCoreRpc.mockImplementation(async (req: { method: string }) => {
        if (req.method === 'openhuman.tools_composio_execute') {
          return {
            successful: true,
            data: { messages: [{ messageText: 'https://www.linkedin.com/in/test-user' }] },
          };
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

      // Wait for Gmail stage to complete and save_profile to start
      await act(async () => {
        await Promise.resolve();
        await Promise.resolve();
      });

      // User continues while save_profile is still running, then the route unmounts.
      fireEvent.click(screen.getByRole('button', { name: /continue to chat/i }));
      expect(onNext).toHaveBeenCalled();
      unmount();

      // Resolve remaining pipeline stages after unmount
      await act(async () => {
        resolveSave({ path: '/tmp/PROFILE.md', bytes: 128 });
        await Promise.resolve();
      });

      // Verify save_profile was called — pipeline continued after unmount
      const saveCalls = callCoreRpc.mock.calls.filter(
        (c: Array<{ method: string }>) => c[0].method === 'openhuman.learning_save_profile'
      );
      expect(saveCalls.length).toBe(1);
      // Apify must never have been invoked.
      const apifyCalls = callCoreRpc.mock.calls.filter(
        (c: Array<{ method: string }>) => c[0].method === 'openhuman.tools_apify_linkedin_scrape'
      );
      expect(apifyCalls.length).toBe(0);
    });
  });

  it('treats Gmail insufficient-scope failures as recoverable and non-blocking', async () => {
    const onNext = vi.fn().mockResolvedValue(undefined);
    callCoreRpc.mockResolvedValueOnce({
      successful: false,
      data: null,
      error: 'Request had insufficient authentication scopes.',
    });

    renderWithProviders(
      <ContextGatheringStep connectedSources={['composio:gmail']} onNext={onNext} />
    );

    await waitFor(() => {
      expect(screen.getByText(/your chat is ready/i)).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole('button', { name: /continue to chat/i }));
    expect(onNext).toHaveBeenCalledTimes(1);
    expect(callCoreRpc).toHaveBeenCalledTimes(1);
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
      expect(screen.getByText(/your chat is ready/i)).toBeInTheDocument();
    });

    expect(screen.getByRole('button', { name: /continue to chat/i })).toBeInTheDocument();
    expect(screen.queryByText('disk full')).not.toBeInTheDocument();

    // fireEvent not needed — onNext is available via the button but user can also
    // just verify the friendly message is shown
  });

  it('captures auto-advance onNext rejection to Sentry with the documented tags (#2081)', async () => {
    vi.useFakeTimers();
    const failure = new Error('app_state_snapshot timed out');
    const onNext = vi.fn().mockRejectedValue(failure);

    renderWithProviders(<ContextGatheringStep connectedSources={['notion']} onNext={onNext} />);

    // No-Gmail branch finishes synchronously, then auto-advance fires after 800ms.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(900);
    });

    expect(onNext).toHaveBeenCalledTimes(1);
    await vi.waitFor(() => expect(captureException).toHaveBeenCalledTimes(1));
    const [thrown, ctx] = captureException.mock.calls[0];
    expect(thrown).toBe(failure);
    expect(ctx).toEqual({ tags: { flow: 'onboarding-complete', step: 'auto-advance' } });

    vi.useRealTimers();
  });

  // --------------------------------------------------------------------------
  // #2156 — slow-but-alive snapshot / save_profile path
  // --------------------------------------------------------------------------
  describe('staged still-working UI (#2156)', () => {
    afterEach(() => {
      vi.useRealTimers();
    });

    it('passes a 90s timeoutMs override on learning_save_profile so slow first launches finish', async () => {
      const onNext = vi.fn().mockResolvedValue(undefined);
      callCoreRpc.mockImplementation(async (req: { method: string }) => {
        if (req.method === 'openhuman.tools_composio_execute') {
          return {
            successful: true,
            data: { messages: [{ messageText: 'https://www.linkedin.com/in/jane-doe' }] },
          };
        }
        if (req.method === 'openhuman.learning_save_profile') {
          return { path: '/tmp/PROFILE.md', bytes: 1 };
        }
        throw new Error(`unexpected RPC ${req.method}`);
      });

      renderWithProviders(
        <ContextGatheringStep connectedSources={['composio:gmail']} onNext={onNext} />
      );

      await waitFor(() => expect(onNext).toHaveBeenCalled(), { timeout: 5000 });

      const saveCall = callCoreRpc.mock.calls.find(
        (c: Array<{ method: string; timeoutMs?: number }>) =>
          c[0].method === 'openhuman.learning_save_profile'
      );
      expect(saveCall![0].timeoutMs).toBeGreaterThan(30_000);
      expect(saveCall![0].timeoutMs).toBeLessThanOrEqual(10 * 60 * 1_000);
    });

    it('swaps to the still-working copy after 30s while the pipeline is still pending', async () => {
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

      // Initial copy — fast happy path.
      expect(screen.getByTestId('context-gathering-title').textContent).toMatch(
        /building your profile/i
      );
      expect(screen.queryByTestId('core-alive-indicator')).not.toBeInTheDocument();

      // Cross the 30s threshold.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(30_500);
      });

      expect(screen.getByTestId('context-gathering-title').textContent).toMatch(/still working/i);
      // Indicator appears so users see whether core is alive or unreachable.
      expect(screen.getByTestId('core-alive-indicator')).toBeInTheDocument();

      await act(async () => {
        resolveGmail({ successful: true, data: { messages: [] } });
      });
    });

    it('reports the core as alive when core.ping returns ok', async () => {
      // Fake timers must be active before render so the 30s still-working
      // setTimeout registers against the fake clock.
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

      // Drive the still-working transition under fake timers, then flip
      // back to real timers so waitFor can poll the React commit for the
      // resolved indicator state (waitFor doesn't observe fake-time
      // microtasks reliably).
      await act(async () => {
        await vi.advanceTimersByTimeAsync(30_500);
      });
      vi.useRealTimers();

      const indicator = await screen.findByTestId('core-alive-indicator');
      await waitFor(() => {
        expect(indicator.getAttribute('data-alive-state')).toBe('alive');
      });
      expect(testCoreRpcConnection).toHaveBeenCalled();
      expect(indicator.textContent ?? '').toMatch(/core is reachable/i);

      await act(async () => {
        resolveGmail({ successful: true, data: { messages: [] } });
      });
    });

    it('treats HTTP 401 as alive (auth not ready yet, core is up)', async () => {
      vi.useFakeTimers();
      testCoreRpcConnection.mockResolvedValue({ ok: false, status: 401 } as Response);

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
        await vi.advanceTimersByTimeAsync(30_500);
      });
      vi.useRealTimers();

      const indicator = await screen.findByTestId('core-alive-indicator');
      await waitFor(() => {
        expect(indicator.getAttribute('data-alive-state')).toBe('alive');
      });

      await act(async () => {
        resolveGmail({ successful: true, data: { messages: [] } });
      });
    });

    it('passes an AbortSignal so a TCP black-hole probe cannot hang forever', async () => {
      vi.useFakeTimers();
      // Hang the probe indefinitely so the only way it can resolve is via
      // the controller-driven abort path. Capture whether the signal was
      // forwarded.
      testCoreRpcConnection.mockImplementation(
        ((_url: string, _token?: string, init?: { signal?: AbortSignal }) =>
          new Promise<Response>((_, reject) => {
            init?.signal?.addEventListener('abort', () =>
              reject(new DOMException('aborted', 'AbortError'))
            );
          })) as never
      );

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

      // Enter still-working state.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(30_500);
      });
      // Drive the per-probe 3s timeout to fire the abort.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(3_500);
      });
      vi.useRealTimers();

      expect(testCoreRpcConnection).toHaveBeenCalled();
      const lastCall = testCoreRpcConnection.mock.calls[
        testCoreRpcConnection.mock.calls.length - 1
      ] as unknown as [string, string | undefined, { signal?: AbortSignal } | undefined];
      expect(lastCall[2]?.signal).toBeInstanceOf(AbortSignal);

      const indicator = await screen.findByTestId('core-alive-indicator');
      await waitFor(() => {
        expect(indicator.getAttribute('data-alive-state')).toBe('unreachable');
      });

      await act(async () => {
        resolveGmail({ successful: true, data: { messages: [] } });
      });
    });

    it('reports unreachable when core.ping rejects', async () => {
      vi.useFakeTimers();
      testCoreRpcConnection.mockRejectedValue(new Error('ECONNREFUSED'));

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
        await vi.advanceTimersByTimeAsync(30_500);
      });
      vi.useRealTimers();

      const indicator = await screen.findByTestId('core-alive-indicator');
      await waitFor(() => {
        expect(indicator.getAttribute('data-alive-state')).toBe('unreachable');
      });
      expect(indicator.textContent ?? '').toMatch(/core is not responding/i);

      await act(async () => {
        resolveGmail({ successful: true, data: { messages: [] } });
      });
    });
  });
});
