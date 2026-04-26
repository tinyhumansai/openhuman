import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import {
  openhumanGetConfig,
  openhumanUpdateModelSettings,
} from '../../../../utils/tauriCommands/config';
import CustomModelSection from '../CustomModelSection';

// Mock the tauri commands module
vi.mock('../../../../utils/tauriCommands/config', () => ({
  openhumanGetConfig: vi.fn(),
  openhumanUpdateModelSettings: vi.fn(),
}));

describe('CustomModelSection', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders the custom provider section with title and description', () => {
    (openhumanGetConfig as ReturnType<typeof vi.fn>).mockResolvedValue({
      result: { config: { api_url: null, api_key: null } },
    });
    renderWithProviders(<CustomModelSection />);
    expect(screen.getByText('Custom Provider (OpenAI Compatible)')).toBeInTheDocument();
    expect(
      screen.getByText(/Configure a custom OpenAI-compatible backend/i)
    ).toBeInTheDocument();
  });

  it('shows loading state while fetching config', () => {
    (openhumanGetConfig as ReturnType<typeof vi.fn>).mockImplementation(
      () => new Promise(() => {}) // never resolves
    );
    renderWithProviders(<CustomModelSection />);
    expect(screen.getByText(/Loading settings/i)).toBeInTheDocument();
  });

  it('renders empty fields when no config exists', async () => {
    (openhumanGetConfig as ReturnType<typeof vi.fn>).mockResolvedValue({
      result: { config: { api_url: null, api_key: null } },
    });
    renderWithProviders(<CustomModelSection />);
    await waitFor(() => {
      expect(screen.getByPlaceholderText('http://localhost:8000/v1')).toHaveValue('');
    });
    expect(screen.getByPlaceholderText('sk-...')).toHaveValue('');
  });

  it('pre-fills fields with stored config', async () => {
    (openhumanGetConfig as ReturnType<typeof vi.fn>).mockResolvedValue({
      result: { config: { api_url: 'http://localhost:9000/v1', api_key: 'sk-test123' } },
    });
    renderWithProviders(<CustomModelSection />);
    await waitFor(() => {
      expect(screen.getByPlaceholderText('http://localhost:8000/v1')).toHaveValue(
        'http://localhost:9000/v1'
      );
    });
    // API key should show masked placeholder when stored
    expect(screen.getByPlaceholderText('••••••••')).toHaveValue('••••••••');
  });

  it('allows editing and saving api_url and api_key', async () => {
    const user = userEvent.setup();
    (openhumanGetConfig as ReturnType<typeof vi.fn>).mockResolvedValue({
      result: { config: { api_url: null, api_key: null } },
    });
    (openhumanUpdateModelSettings as ReturnType<typeof vi.fn>).mockResolvedValue(undefined);

    renderWithProviders(<CustomModelSection />);
    await waitFor(() => {
      expect(screen.queryByText(/Loading settings/i)).not.toBeInTheDocument();
    });

    const urlInput = screen.getByPlaceholderText('http://localhost:8000/v1');
    await user.clear(urlInput);
    await user.type(urlInput, 'http://my-server:8080/v1');

    const keyInput = screen.getByPlaceholderText('sk-...');
    await user.clear(keyInput);
    await user.type(keyInput, 'sk-mynewkey');

    await user.click(screen.getByRole('button', { name: 'Save Config' }));

    await waitFor(() => {
      expect(openhumanUpdateModelSettings).toHaveBeenCalledWith({
        api_url: 'http://my-server:8080/v1',
        api_key: 'sk-mynewkey',
      });
    });
  });

  it('shows success message after saving', async () => {
    const user = userEvent.setup();
    (openhumanGetConfig as ReturnType<typeof vi.fn>).mockResolvedValue({
      result: { config: { api_url: null, api_key: null } },
    });
    (openhumanUpdateModelSettings as ReturnType<typeof vi.fn>).mockResolvedValue(undefined);

    renderWithProviders(<CustomModelSection />);
    await waitFor(() => {
      expect(screen.queryByText(/Loading settings/i)).not.toBeInTheDocument();
    });

    await user.click(screen.getByRole('button', { name: 'Save Config' }));

    await waitFor(() => {
      expect(screen.getByText('Saved successfully.')).toBeInTheDocument();
    });
  });

  it('shows error message when save fails', async () => {
    const user = userEvent.setup();
    (openhumanGetConfig as ReturnType<typeof vi.fn>).mockResolvedValue({
      result: { config: { api_url: null, api_key: null } },
    });
    (openhumanUpdateModelSettings as ReturnType<typeof vi.fn>).mockRejectedValue(
      new Error('Update failed')
    );

    renderWithProviders(<CustomModelSection />);
    await waitFor(() => {
      expect(screen.queryByText(/Loading settings/i)).not.toBeInTheDocument();
    });

    await user.click(screen.getByRole('button', { name: 'Save Config' }));

    await waitFor(() => {
      expect(screen.getByText(/Failed to save custom backend settings/i)).toBeInTheDocument();
    });
  });
});
