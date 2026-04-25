import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import ContextGatheringStep from '../ContextGatheringStep';

const callCoreRpc = vi.hoisted(() => vi.fn());
vi.mock('../../../../services/coreRpcClient', () => ({ callCoreRpc }));

const invoke = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

// WebviewHost reads from the redux accounts slice; the offscreen mount
// in ContextGatheringStep is purely a side-effect-free webview anchor
// for these tests.
vi.mock('../../../../components/accounts/WebviewHost', () => ({
  default: () => null,
}));

describe('ContextGatheringStep', () => {
  beforeEach(() => {
    callCoreRpc.mockReset();
    invoke.mockReset();
  });

  it('renders user-driven intro gate with a Skip and privacy link, does NOT auto-run pipeline', () => {
    renderWithProviders(
      <ContextGatheringStep connectedSources={[]} onNext={() => Promise.resolve()} />
    );

    expect(screen.getByTestId('context-gathering-intro')).toBeInTheDocument();
    expect(screen.getByRole('heading', { level: 1, name: /getting to know you/i }))
      .toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Skip for now' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'What leaves my computer?' })).toBeInTheDocument();
    expect(callCoreRpc).not.toHaveBeenCalled();
    expect(invoke).not.toHaveBeenCalled();
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
    expect(invoke).not.toHaveBeenCalled();
  });

  it('Skip for now invokes onNext without starting the pipeline', () => {
    const onNext = vi.fn().mockResolvedValue(undefined);
    renderWithProviders(
      <ContextGatheringStep
        connectedSources={['webview:gmail']}
        gmailAccountId="acct-1"
        onNext={onNext}
      />
    );

    fireEvent.click(screen.getByRole('button', { name: 'Skip for now' }));
    expect(onNext).toHaveBeenCalledTimes(1);
    expect(callCoreRpc).not.toHaveBeenCalled();
    expect(invoke).not.toHaveBeenCalled();
  });

  it('with Gmail webview connected: Start runs the webview gmail helper, then enrichment with the URL', async () => {
    invoke.mockResolvedValue('https://www.linkedin.com/in/jane-doe');
    callCoreRpc.mockResolvedValue({
      profile_url: 'https://www.linkedin.com/in/jane-doe',
      profile_data: null,
      log: [],
    });
    renderWithProviders(
      <ContextGatheringStep
        connectedSources={['webview:gmail']}
        gmailAccountId="acct-1"
        onNext={() => Promise.resolve()}
      />
    );

    fireEvent.click(screen.getByRole('button', { name: "Let's go!" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith('gmail_find_linkedin_profile_url', {
        accountId: 'acct-1',
      });
    });
    await waitFor(() => {
      expect(callCoreRpc).toHaveBeenCalledTimes(1);
    });
    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.learning_linkedin_enrichment',
      params: { profile_url: 'https://www.linkedin.com/in/jane-doe' },
    });
  });

  it('webview helper finds no URL: pipeline skips enrichment and finishes', async () => {
    invoke.mockResolvedValue(null);
    renderWithProviders(
      <ContextGatheringStep
        connectedSources={['webview:gmail']}
        gmailAccountId="acct-1"
        onNext={() => Promise.resolve()}
      />
    );

    fireEvent.click(screen.getByRole('button', { name: "Let's go!" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledTimes(1);
    });
    // No URL → core enrichment not invoked.
    expect(callCoreRpc).not.toHaveBeenCalled();
    await waitFor(() => {
      expect(screen.getByText('Context Ready')).toBeInTheDocument();
    });
  });
});
