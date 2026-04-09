import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import Home from '../Home';

vi.mock('../../components/ConnectionIndicator', () => ({
  default: () => <div>Connection Indicator</div>,
}));

vi.mock('../../hooks/useUser', () => ({ useUser: () => ({ user: { firstName: 'Tester' } }) }));

vi.mock('../../utils/localAiBootstrap', () => ({
  bootstrapLocalAiWithRecommendedPreset: vi.fn(),
  ensureRecommendedLocalAiPresetIfNeeded: vi.fn(),
  triggerLocalAiAssetBootstrap: vi.fn(),
}));

vi.mock('../../utils/tauriCommands', () => ({
  isTauri: vi.fn(() => true),
  openhumanLocalAiStatus: vi.fn(),
}));

describe('Home bootstrap button states', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('shows "Running" badge instead of Bootstrap button when state is ready', async () => {
    const tauriCommands = await import('../../utils/tauriCommands');
    const bootstrapUtils = await import('../../utils/localAiBootstrap');

    vi.mocked(tauriCommands.openhumanLocalAiStatus).mockResolvedValue({
      result: { state: 'ready', model_id: 'gemma3:4b-it-qat' } as never,
      logs: [],
    });
    vi.mocked(bootstrapUtils.ensureRecommendedLocalAiPresetIfNeeded).mockResolvedValue({
      presets: {} as never,
      recommendedTier: 'high',
      selectedTier: 'high',
      hadSelectedTier: true,
      appliedTier: null,
    });

    renderWithProviders(<Home />);

    await waitFor(() => {
      expect(screen.getByText('Running')).toBeInTheDocument();
    });

    expect(screen.queryByRole('button', { name: 'Bootstrap' })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Re-bootstrap' })).toBeInTheDocument();
  });

  it('shows "Retry" button when state is degraded', async () => {
    const tauriCommands = await import('../../utils/tauriCommands');
    const bootstrapUtils = await import('../../utils/localAiBootstrap');

    // Keep returning degraded so the auto-retry doesn't change the visible state
    vi.mocked(tauriCommands.openhumanLocalAiStatus).mockResolvedValue({
      result: {
        state: 'degraded',
        model_id: 'gemma3:4b-it-qat',
        warning: 'Ollama not found',
      } as never,
      logs: [],
    });
    vi.mocked(bootstrapUtils.ensureRecommendedLocalAiPresetIfNeeded).mockResolvedValue({
      presets: {} as never,
      recommendedTier: 'high',
      selectedTier: 'high',
      hadSelectedTier: true,
      appliedTier: null,
    });
    // The Home component auto-retries on degraded — let it resolve without changing state
    vi.mocked(bootstrapUtils.bootstrapLocalAiWithRecommendedPreset).mockResolvedValue({
      preset: {} as never,
      download: {} as never,
    });

    renderWithProviders(<Home />);

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Retry' })).toBeInTheDocument();
    });

    expect(screen.queryByText('Running')).not.toBeInTheDocument();
  });

  it('shows "Bootstrap" button when state is idle', async () => {
    const tauriCommands = await import('../../utils/tauriCommands');
    const bootstrapUtils = await import('../../utils/localAiBootstrap');

    vi.mocked(tauriCommands.openhumanLocalAiStatus).mockResolvedValue({
      result: { state: 'idle', model_id: 'gemma3:4b-it-qat' } as never,
      logs: [],
    });
    vi.mocked(bootstrapUtils.ensureRecommendedLocalAiPresetIfNeeded).mockResolvedValue({
      presets: {} as never,
      recommendedTier: 'high',
      selectedTier: 'high',
      hadSelectedTier: true,
      appliedTier: null,
    });

    renderWithProviders(<Home />);

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Bootstrap' })).toBeInTheDocument();
    });

    expect(screen.queryByText('Running')).not.toBeInTheDocument();
  });

  it('shows success message after re-bootstrap completes', async () => {
    const tauriCommands = await import('../../utils/tauriCommands');
    const bootstrapUtils = await import('../../utils/localAiBootstrap');

    vi.mocked(tauriCommands.openhumanLocalAiStatus)
      .mockResolvedValueOnce({
        result: { state: 'ready', model_id: 'gemma3:4b-it-qat' } as never,
        logs: [],
      })
      .mockResolvedValue({
        result: { state: 'ready', model_id: 'gemma3:4b-it-qat' } as never,
        logs: [],
      });
    vi.mocked(bootstrapUtils.ensureRecommendedLocalAiPresetIfNeeded).mockResolvedValue({
      presets: {} as never,
      recommendedTier: 'high',
      selectedTier: 'high',
      hadSelectedTier: true,
      appliedTier: null,
    });
    vi.mocked(bootstrapUtils.bootstrapLocalAiWithRecommendedPreset).mockResolvedValue({
      preset: {} as never,
      download: {} as never,
    });

    renderWithProviders(<Home />);

    await waitFor(() => {
      expect(screen.getByText('Running')).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole('button', { name: 'Re-bootstrap' }));

    await waitFor(() => {
      expect(screen.getByText('Re-bootstrap complete')).toBeInTheDocument();
    });
  });
});
