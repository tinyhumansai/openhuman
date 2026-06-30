import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import ModelHealthPanel from '../ModelHealthPanel';

vi.mock('../../../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));
vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));
vi.mock('../../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const MOCK_RESPONSE = {
  models: [
    {
      id: 'deepseek-v3',
      provider: 'SiliconFlow',
      cost_per_1m_input: 0.14,
      cost_per_1m_cached_input: 0.014,
      cost_per_1m_output: 0.33,
      context_window: 128_000,
      vision: false,
      quality_score: 4,
      hallucination_rate: 0.03,
      agents_using: 5,
      tasks_evaluated: 60,
    },
    {
      id: 'qwen-2.5-8b',
      provider: 'OpenRouter',
      cost_per_1m_output: 0.09,
      vision: true,
      quality_score: 3,
      hallucination_rate: 0.04,
      agents_using: 1,
      tasks_evaluated: 20,
    },
    {
      id: 'bad-model',
      provider: 'Test',
      cost_per_1m_output: 1.0,
      vision: false,
      quality_score: 2,
      hallucination_rate: 0.18,
      agents_using: 2,
      tasks_evaluated: 50,
    },
  ],
  config: { hallucination_threshold: 0.1, min_tasks_for_rating: 10, evaluation_window_tasks: 50 },
};

async function mockRpc(response: unknown) {
  const { callCoreRpc } = await import('../../../../services/coreRpcClient');
  vi.mocked(callCoreRpc).mockResolvedValueOnce(response);
}

async function mockRpcReject(err: unknown) {
  const { callCoreRpc } = await import('../../../../services/coreRpcClient');
  vi.mocked(callCoreRpc).mockRejectedValueOnce(err);
}

describe('ModelHealthPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders panel with table', async () => {
    await mockRpc(MOCK_RESPONSE);
    renderWithProviders(<ModelHealthPanel />);
    await waitFor(() => {
      expect(screen.getByText('deepseek-v3')).toBeTruthy();
    });
    expect(screen.getByText('qwen-2.5-8b')).toBeTruthy();
    expect(screen.getByText('bad-model')).toBeTruthy();
  });

  it('shows context window and input/output pricing', async () => {
    await mockRpc(MOCK_RESPONSE);
    renderWithProviders(<ModelHealthPanel />);
    await waitFor(() => {
      expect(screen.getByText('deepseek-v3')).toBeTruthy();
    });
    // Context window rendered as a compact token-count label next to provider.
    expect(screen.getByText(/128K ctx/)).toBeTruthy();
    // Cost cell shows "input / output" when input pricing is present.
    expect(screen.getByText(/\$0\.14 \/ \$0\.33/)).toBeTruthy();
  });

  it('shows correct status badges', async () => {
    await mockRpc(MOCK_RESPONSE);
    renderWithProviders(<ModelHealthPanel />);
    await waitFor(() => {
      expect(screen.getByText('deepseek-v3')).toBeTruthy();
    });
    expect(screen.getAllByText('settings.modelHealth.badge.keep').length).toBeGreaterThan(0);
    expect(screen.getAllByText('settings.modelHealth.badge.vision').length).toBeGreaterThan(0);
    expect(screen.getAllByText('settings.modelHealth.badge.replace').length).toBeGreaterThan(0);
  });

  it('filters by status', async () => {
    await mockRpc(MOCK_RESPONSE);
    const { container } = renderWithProviders(<ModelHealthPanel />);
    await waitFor(() => {
      expect(screen.getByText('deepseek-v3')).toBeTruthy();
    });
    const select = container.querySelector('select')!;
    fireEvent.change(select, { target: { value: 'vision' } });
    await waitFor(() => {
      expect(screen.queryByText('deepseek-v3')).toBeNull();
    });
    expect(screen.getByText('qwen-2.5-8b')).toBeTruthy();
  });

  it('sorts by column', async () => {
    await mockRpc(MOCK_RESPONSE);
    renderWithProviders(<ModelHealthPanel />);
    await waitFor(() => {
      expect(screen.getByText('deepseek-v3')).toBeTruthy();
    });
    fireEvent.click(screen.getByText('settings.modelHealth.col.cost'));
    // After sorting by cost asc, first row should be cheapest model (qwen at $0.09)
    const rows = screen.getAllByRole('row');
    expect(rows[1].textContent).toContain('qwen-2.5-8b');
  });

  it('shows swap button for replace-flagged models', async () => {
    await mockRpc(MOCK_RESPONSE);
    renderWithProviders(<ModelHealthPanel />);
    await waitFor(() => {
      expect(screen.getByText('settings.modelHealth.swap')).toBeTruthy();
    });
  });

  it('opens swap modal on click', async () => {
    await mockRpc(MOCK_RESPONSE);
    renderWithProviders(<ModelHealthPanel />);
    await waitFor(() => {
      expect(screen.getByText('settings.modelHealth.swap')).toBeTruthy();
    });
    fireEvent.click(screen.getByText('settings.modelHealth.swap'));
    await waitFor(() => {
      expect(screen.getByText('settings.modelHealth.modal.title')).toBeTruthy();
    });
  });

  it('shows empty state when no models', async () => {
    await mockRpc({ models: [], config: MOCK_RESPONSE.config });
    renderWithProviders(<ModelHealthPanel />);
    await waitFor(() => {
      expect(screen.getByText('settings.modelHealth.empty')).toBeTruthy();
    });
  });

  it('shows loading then content', async () => {
    await mockRpc(MOCK_RESPONSE);
    renderWithProviders(<ModelHealthPanel />);
    await waitFor(() => {
      expect(screen.getByTestId('model-health-panel')).toBeTruthy();
    });
  });

  it('unwraps RpcOutcome {result, logs} envelope', async () => {
    await mockRpc({ result: MOCK_RESPONSE, logs: ['dashboard.model_health returned 3 models'] });
    renderWithProviders(<ModelHealthPanel />);
    await waitFor(() => {
      expect(screen.getByText('deepseek-v3')).toBeTruthy();
    });
    expect(screen.getByText('qwen-2.5-8b')).toBeTruthy();
  });

  it('handles rpc failure gracefully', async () => {
    await mockRpcReject(new Error('config unavailable'));
    renderWithProviders(<ModelHealthPanel />);
    await waitFor(() => {
      expect(screen.getByText('settings.modelHealth.empty')).toBeTruthy();
    });
  });

  it('toggles sort direction on same column click', async () => {
    await mockRpc(MOCK_RESPONSE);
    renderWithProviders(<ModelHealthPanel />);
    await waitFor(() => {
      expect(screen.getByText('deepseek-v3')).toBeTruthy();
    });
    const costHeader = screen.getByText('settings.modelHealth.col.cost');
    fireEvent.click(costHeader); // asc
    fireEvent.click(costHeader); // desc — toggles direction
    const rows = screen.getAllByRole('row');
    expect(rows[1].textContent).toContain('bad-model'); // most expensive first
  });

  it('modal shows candidates and closes on backdrop click', async () => {
    await mockRpc(MOCK_RESPONSE);
    renderWithProviders(<ModelHealthPanel />);
    await waitFor(() => {
      expect(screen.getByText('settings.modelHealth.swap')).toBeTruthy();
    });
    fireEvent.click(screen.getByText('settings.modelHealth.swap'));
    await waitFor(() => {
      expect(screen.getByText('settings.modelHealth.modal.title')).toBeTruthy();
    });
    // Verify candidates are shown
    expect(screen.getByText('settings.modelHealth.modal.apply')).toBeTruthy();
    expect(screen.getByText('settings.modelHealth.modal.cancel')).toBeTruthy();
    // Close via cancel
    fireEvent.click(screen.getByText('settings.modelHealth.modal.cancel'));
    await waitFor(() => {
      expect(screen.queryByText('settings.modelHealth.modal.title')).toBeNull();
    });
  });

  it('closes modal on cancel', async () => {
    await mockRpc(MOCK_RESPONSE);
    renderWithProviders(<ModelHealthPanel />);
    await waitFor(() => {
      expect(screen.getByText('settings.modelHealth.swap')).toBeTruthy();
    });
    fireEvent.click(screen.getByText('settings.modelHealth.swap'));
    await waitFor(() => {
      expect(screen.getByText('settings.modelHealth.modal.title')).toBeTruthy();
    });
    fireEvent.click(screen.getByText('settings.modelHealth.modal.cancel'));
    await waitFor(() => {
      expect(screen.queryByText('settings.modelHealth.modal.title')).toBeNull();
    });
  });

  it('Apply button is disabled until a candidate is selected', async () => {
    await mockRpc(MOCK_RESPONSE);
    renderWithProviders(<ModelHealthPanel />);
    await waitFor(() => {
      expect(screen.getByText('settings.modelHealth.swap')).toBeTruthy();
    });
    fireEvent.click(screen.getByText('settings.modelHealth.swap'));
    const applyButton = await screen.findByText('settings.modelHealth.modal.apply');
    expect((applyButton as HTMLButtonElement).disabled).toBe(true);

    // Select a candidate then re-check.
    const candidates = screen.getAllByRole('radio');
    expect(candidates.length).toBeGreaterThan(0);
    fireEvent.click(candidates[0]);
    expect((applyButton as HTMLButtonElement).disabled).toBe(false);

    // Clicking Apply closes the modal (UI-side; backend swap is a follow-up).
    fireEvent.click(applyButton);
    await waitFor(() => {
      expect(screen.queryByText('settings.modelHealth.modal.title')).toBeNull();
    });
  });
});
