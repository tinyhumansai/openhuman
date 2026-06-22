import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, test, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';

const hoisted = vi.hoisted(() => ({ callCoreRpc: vi.fn() }));

vi.mock('../../../../services/coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => hoisted.callCoreRpc(...args),
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));

describe('AgentBoxPanel', () => {
  test('renders provider wiring when configured and enabled', async () => {
    hoisted.callCoreRpc.mockReset();
    hoisted.callCoreRpc.mockResolvedValue({
      mode_enabled: true,
      provider_configured: true,
      provider: {
        slug: 'gmi-maas',
        base_url: 'https://api.gmi-serving.com',
        model: 'deepseek-ai/DeepSeek-V4-Pro',
      },
    });

    const Panel = (await import('../AgentBoxPanel')).default;
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.getByText('gmi-maas')).toBeInTheDocument();
    });
    expect(screen.getByText('https://api.gmi-serving.com')).toBeInTheDocument();
    expect(screen.getByText('deepseek-ai/DeepSeek-V4-Pro')).toBeInTheDocument();
    expect(hoisted.callCoreRpc).toHaveBeenCalledWith(
      expect.objectContaining({ method: 'openhuman.agentbox_status' })
    );
  });

  test('renders not-configured message when provider absent', async () => {
    hoisted.callCoreRpc.mockReset();
    hoisted.callCoreRpc.mockResolvedValue({
      mode_enabled: false,
      provider_configured: false,
      provider: null,
    });

    const Panel = (await import('../AgentBoxPanel')).default;
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.getByText(/GMI_MAAS_BASE_URL/)).toBeInTheDocument();
    });
  });

  test('renders unavailable card when the RPC throws', async () => {
    hoisted.callCoreRpc.mockReset();
    hoisted.callCoreRpc.mockRejectedValue(new Error('rpc transport unavailable'));

    const Panel = (await import('../AgentBoxPanel')).default;
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.getByText(/rpc transport unavailable/i)).toBeInTheDocument();
    });
  });

  test('refresh button re-fetches status', async () => {
    hoisted.callCoreRpc.mockReset();
    hoisted.callCoreRpc.mockResolvedValue({
      mode_enabled: true,
      provider_configured: false,
      provider: null,
    });

    const Panel = (await import('../AgentBoxPanel')).default;
    renderWithProviders(<Panel />);

    await waitFor(() => expect(hoisted.callCoreRpc).toHaveBeenCalledTimes(1));

    const refresh = await screen.findByRole('button', { name: /refresh/i });
    await userEvent.click(refresh);

    await waitFor(() => expect(hoisted.callCoreRpc).toHaveBeenCalledTimes(2));
  });
});
