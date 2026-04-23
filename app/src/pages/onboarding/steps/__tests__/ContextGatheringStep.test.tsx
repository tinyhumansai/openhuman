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
      screen.getByRole('heading', { level: 1, name: 'Getting to know you' })
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Skip for now' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'What leaves my computer?' })).toBeInTheDocument();
    expect(callCoreRpc).not.toHaveBeenCalled();
  });

  it('no-Gmail branch: Start marks all stages skipped and surfaces Continue without any RPC', async () => {
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
    renderWithProviders(<ContextGatheringStep connectedSources={['gmail']} onNext={onNext} />);

    fireEvent.click(screen.getByRole('button', { name: 'Skip for now' }));
    expect(onNext).toHaveBeenCalledTimes(1);
    expect(callCoreRpc).not.toHaveBeenCalled();
  });

  it('with Gmail connected: Start when ready triggers the pipeline exactly once', async () => {
    callCoreRpc.mockResolvedValue({ profile_url: null, profile_data: null, log: [] });
    renderWithProviders(
      <ContextGatheringStep connectedSources={['gmail']} onNext={() => Promise.resolve()} />
    );

    fireEvent.click(screen.getByRole('button', { name: 'Start when ready' }));

    await waitFor(() => {
      expect(callCoreRpc).toHaveBeenCalledTimes(1);
    });
    expect(callCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.learning_linkedin_enrichment' });
  });
});
