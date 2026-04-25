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

  it('with Gmail connected: Start runs the core enrichment pipeline', async () => {
    callCoreRpc.mockResolvedValue({
      profile_url: 'https://www.linkedin.com/in/jane-doe',
      profile_data: null,
      log: [
        'Found LinkedIn profile: https://www.linkedin.com/in/jane-doe',
        'profile scraped successfully',
        'PROFILE.md written',
      ],
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
    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.learning_linkedin_enrichment',
      params: {},
    });
    await waitFor(() => {
      expect(screen.getByText('Context Ready')).toBeInTheDocument();
    });
  });
});
