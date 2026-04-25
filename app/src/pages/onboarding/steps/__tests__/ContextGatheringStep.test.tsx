import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import ContextGatheringStep from '../ContextGatheringStep';

const callCoreRpc = vi.hoisted(() => vi.fn());
vi.mock('../../../../services/coreRpcClient', () => ({ callCoreRpc }));

describe('ContextGatheringStep', () => {
  beforeEach(() => {
    callCoreRpc.mockReset();
  });

  it('renders user-driven intro gate with a Skip and privacy link, does NOT auto-run pipeline', () => {
    renderWithProviders(
      <ContextGatheringStep connectedSources={[]} onNext={() => Promise.resolve()} />
    );

    expect(screen.getByTestId('context-gathering-intro')).toBeInTheDocument();
    expect(
      screen.getByRole('heading', { level: 1, name: /getting to know you/i })
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Skip for now' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'What leaves my computer?' })).toBeInTheDocument();
    expect(callCoreRpc).not.toHaveBeenCalled();
  });

  it('no-Gmail branch: Continue marks all stages skipped without any RPC', async () => {
    renderWithProviders(
      <ContextGatheringStep connectedSources={['notion']} onNext={() => Promise.resolve()} />
    );

    expect(screen.getByText(/haven't connected Gmail/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Continue' })).not.toBeDisabled();
    });
    expect(screen.getByText('Context Ready')).toBeInTheDocument();
    expect(callCoreRpc).not.toHaveBeenCalled();
  });

  it('Skip for now invokes onNext without starting the pipeline', () => {
    const onNext = vi.fn().mockResolvedValue(undefined);
    renderWithProviders(
      <ContextGatheringStep connectedSources={['composio:gmail']} onNext={onNext} />
    );

    fireEvent.click(screen.getByRole('button', { name: 'Skip for now' }));
    expect(onNext).toHaveBeenCalledTimes(1);
    expect(callCoreRpc).not.toHaveBeenCalled();
  });

  it('runs the full Gmail -> Apify scrape -> save pipeline', async () => {
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
      <ContextGatheringStep
        connectedSources={['composio:gmail']}
        onNext={() => Promise.resolve()}
      />
    );

    fireEvent.click(screen.getByRole('button', { name: "Let's go!" }));

    await waitFor(() => {
      expect(screen.getByText('Context Ready')).toBeInTheDocument();
    });

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

  it('skips downstream stages when Gmail finds no LinkedIn URL', async () => {
    callCoreRpc.mockResolvedValueOnce({
      successful: true,
      data: { messages: [{ messageText: 'Hello, no linkedin link here.' }] },
    });

    renderWithProviders(
      <ContextGatheringStep
        connectedSources={['composio:gmail']}
        onNext={() => Promise.resolve()}
      />
    );

    fireEvent.click(screen.getByRole('button', { name: "Let's go!" }));

    await waitFor(() => {
      expect(callCoreRpc).toHaveBeenCalledTimes(1);
    });
    await waitFor(() => {
      expect(screen.getByText('Context Ready')).toBeInTheDocument();
    });
    expect(callCoreRpc).toHaveBeenCalledTimes(1);
  });

  it('surfaces a build-profile error when learning_save_profile rejects', async () => {
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
      <ContextGatheringStep
        connectedSources={['composio:gmail']}
        onNext={() => Promise.resolve()}
      />
    );

    fireEvent.click(screen.getByRole('button', { name: "Let's go!" }));

    await waitFor(() => {
      expect(screen.getByText('Context Ready')).toBeInTheDocument();
    });
    expect(screen.getByText('disk full')).toBeInTheDocument();
  });
});
