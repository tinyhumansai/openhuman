import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import {
  openhumanGetClientConfig,
  openhumanUpdateModelSettings,
} from '../../../../utils/tauriCommands';
import BackendProviderPanel from '../BackendProviderPanel';

vi.mock('../../../../utils/tauriCommands', () => ({
  openhumanGetClientConfig: vi.fn(),
  openhumanUpdateModelSettings: vi.fn(),
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));

function mockClientConfig(overrides: Record<string, unknown> = {}) {
  vi.mocked(openhumanGetClientConfig).mockResolvedValue({
    result: {
      api_url: '',
      default_model: '',
      app_version: '0.0.0-test',
      api_key_set: false,
      ...overrides,
    },
    messages: [],
  } as unknown as Awaited<ReturnType<typeof openhumanGetClientConfig>>);
}

function mockUpdateOk() {
  vi.mocked(openhumanUpdateModelSettings).mockResolvedValue({
    result: { config: {}, workspace_dir: '', config_path: '' },
    messages: [],
  } as unknown as Awaited<ReturnType<typeof openhumanUpdateModelSettings>>);
}

describe('BackendProviderPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockUpdateOk();
  });

  it('renders all provider preset chips and the OpenHuman success banner by default', async () => {
    mockClientConfig();
    renderWithProviders(<BackendProviderPanel />);

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /OpenHuman/i })).toBeTruthy();
    });

    for (const label of ['OpenHuman', 'OpenAI', 'Anthropic', 'OpenRouter', 'Ollama', 'Custom']) {
      expect(screen.getByRole('button', { name: new RegExp(label, 'i') })).toBeTruthy();
    }

    // Congrats banner is visible because OpenHuman is the default
    expect(screen.getByText(/Congrats! You.{1,3}re using the most optimized setup/i)).toBeTruthy();
    // URL input is hidden for OpenHuman
    expect(screen.queryByLabelText(/API URL/i)).toBeNull();
    // API key field is hidden for OpenHuman
    expect(screen.queryByLabelText(/API Key/i)).toBeNull();
    // No role inputs for OpenHuman
    expect(screen.queryByLabelText(/Reasoning/i)).toBeNull();
  });

  it('shows API key + role-model inputs when a non-OpenHuman preset is picked', async () => {
    mockClientConfig();
    renderWithProviders(<BackendProviderPanel />);

    const openaiChip = await screen.findByRole('button', { name: /^OpenAI$/i });
    fireEvent.click(openaiChip);

    // API key field appears
    expect(await screen.findByLabelText(/API Key/i)).toBeTruthy();
    // Role inputs appear
    expect(screen.getByLabelText(/Reasoning/i)).toBeTruthy();
    expect(screen.getByLabelText(/Agentic/i)).toBeTruthy();
    expect(screen.getByLabelText(/Coding/i)).toBeTruthy();
    expect(screen.getByLabelText(/Summarization/i)).toBeTruthy();
    // Congrats banner is gone
    expect(screen.queryByText(/Congrats!/i)).toBeNull();
    // URL field still hidden (only shown for Custom)
    expect(screen.queryByLabelText(/API URL/i)).toBeNull();
  });

  it('reveals the URL input only when Custom is picked', async () => {
    mockClientConfig();
    renderWithProviders(<BackendProviderPanel />);

    const customChip = await screen.findByRole('button', { name: /^Custom$/i });
    fireEvent.click(customChip);

    expect(await screen.findByLabelText(/API URL/i)).toBeTruthy();
  });

  it('Save sends model_routes derived from the selected preset and api_key when touched', async () => {
    mockClientConfig();
    renderWithProviders(<BackendProviderPanel />);

    fireEvent.click(await screen.findByRole('button', { name: /^Anthropic$/i }));

    const keyInput = await screen.findByLabelText(/API Key/i);
    fireEvent.change(keyInput, { target: { value: 'sk-ant-test-123' } });

    fireEvent.click(screen.getByRole('button', { name: /^Save$/i }));

    await waitFor(() => {
      expect(openhumanUpdateModelSettings).toHaveBeenCalled();
    });
    const args = vi.mocked(openhumanUpdateModelSettings).mock.calls[0][0];
    expect(args.api_url).toBe('https://api.anthropic.com/v1/chat/completions');
    expect(args.api_key).toBe('sk-ant-test-123');
    expect(args.model_routes).toBeInstanceOf(Array);
    const hints = (args.model_routes ?? []).map(r => r.hint).sort();
    expect(hints).toEqual(['agentic', 'coding', 'reasoning', 'summarization']);
  });

  it('Save sends model_routes:[] and no api_key when switching back to OpenHuman', async () => {
    mockClientConfig({ api_url: 'https://api.openai.com/v1/chat/completions', api_key_set: true });
    renderWithProviders(<BackendProviderPanel />);

    // Hydration finished — switch to OpenHuman explicitly
    fireEvent.click(await screen.findByRole('button', { name: /^OpenHuman$/i }));
    fireEvent.click(screen.getByRole('button', { name: /^Save$/i }));

    await waitFor(() => expect(openhumanUpdateModelSettings).toHaveBeenCalled());
    const args = vi.mocked(openhumanUpdateModelSettings).mock.calls[0][0];
    expect(args.model_routes).toEqual([]);
    expect(args.api_key).toBeUndefined();
  });

  it('omits all touched fields on Save when nothing was edited (post failed-load safety)', async () => {
    mockClientConfig({ api_url: 'https://api.openai.com/v1/chat/completions', api_key_set: true });
    renderWithProviders(<BackendProviderPanel />);

    // Just hit Save without touching anything
    fireEvent.click(await screen.findByRole('button', { name: /^Save$/i }));

    await waitFor(() => expect(openhumanUpdateModelSettings).toHaveBeenCalled());
    const args = vi.mocked(openhumanUpdateModelSettings).mock.calls[0][0];
    expect(args.api_url).toBeUndefined();
    expect(args.api_key).toBeUndefined();
    expect(args.model_routes).toBeUndefined();
  });

  it('surfaces a "Clear stored key" button only when api_key_set is true (non-OpenHuman)', async () => {
    mockClientConfig({ api_url: 'https://api.openai.com/v1/chat/completions', api_key_set: true });
    renderWithProviders(<BackendProviderPanel />);

    // Wait for the OpenAI preset to be active (api_url matches)
    await waitFor(() => {
      expect(screen.getByLabelText(/API Key/i)).toBeTruthy();
    });
    expect(screen.getByRole('button', { name: /Clear stored key/i })).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: /Clear stored key/i }));
    await waitFor(() => expect(openhumanUpdateModelSettings).toHaveBeenCalled());
    const args = vi.mocked(openhumanUpdateModelSettings).mock.calls[0][0];
    expect(args.api_key).toBe('');
  });

  it('shows an error status when the save RPC rejects', async () => {
    mockClientConfig();
    vi.mocked(openhumanUpdateModelSettings).mockRejectedValueOnce(new Error('boom'));
    renderWithProviders(<BackendProviderPanel />);

    fireEvent.click(await screen.findByRole('button', { name: /^OpenAI$/i }));
    fireEvent.click(screen.getByRole('button', { name: /^Save$/i }));

    await waitFor(() => {
      expect(screen.getByText(/Failed to save: boom/i)).toBeTruthy();
    });
  });

  it('shows a load-error status when the initial client-config fetch rejects', async () => {
    vi.mocked(openhumanGetClientConfig).mockRejectedValueOnce(new Error('offline'));
    renderWithProviders(<BackendProviderPanel />);

    await waitFor(() => {
      expect(screen.getByText(/Failed to load current settings: offline/i)).toBeTruthy();
    });
  });
});
