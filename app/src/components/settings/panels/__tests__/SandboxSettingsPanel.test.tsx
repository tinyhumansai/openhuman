import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import {
  isTauri,
  openhumanGetSandboxSettings,
  openhumanUpdateSandboxSettings,
  type SandboxSettings,
} from '../../../../utils/tauriCommands';
import SandboxSettingsPanel from '../SandboxSettingsPanel';

const sandboxSettings = (overrides: Partial<SandboxSettings> = {}): SandboxSettings => ({
  enabled: true,
  backend: 'auto',
  docker_image: 'alpine:3.20',
  docker_memory_limit_mb: 512,
  docker_cpu_limit: 1.0,
  docker_available: true,
  detected_backend: 'seatbelt',
  env_passthrough: ['PATH', 'HOME', 'TERM'],
  ...overrides,
});

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToSettings: vi.fn(),
    breadcrumbs: [],
  }),
}));

vi.mock('../../../../utils/tauriCommands', async () => {
  const actual = await vi.importActual<typeof import('../../../../utils/tauriCommands')>(
    '../../../../utils/tauriCommands'
  );
  return {
    ...actual,
    isTauri: vi.fn(() => true),
    openhumanGetSandboxSettings: vi.fn(),
    openhumanUpdateSandboxSettings: vi.fn(),
  };
});

const mockGet = vi.mocked(openhumanGetSandboxSettings);
const mockUpdate = vi.mocked(openhumanUpdateSandboxSettings);

describe('SandboxSettingsPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(isTauri).mockReturnValue(true);
    mockGet.mockResolvedValue({ result: sandboxSettings(), logs: [] });
    mockUpdate.mockResolvedValue({ result: {} as never, logs: [] });
  });

  it('loads settings on mount and renders status section', async () => {
    renderWithProviders(<SandboxSettingsPanel />);
    await waitFor(() => expect(mockGet).toHaveBeenCalledTimes(1));
    expect(await screen.findByText('Available')).toBeInTheDocument();
    expect(screen.getByText('seatbelt')).toBeInTheDocument();
  });

  it('shows unavailable status when Docker is not available', async () => {
    mockGet.mockResolvedValue({ result: sandboxSettings({ docker_available: false }), logs: [] });
    renderWithProviders(<SandboxSettingsPanel />);
    expect(await screen.findByText('Unavailable')).toBeInTheDocument();
  });

  it('renders the backend dropdown with current selection', async () => {
    renderWithProviders(<SandboxSettingsPanel />);
    await waitFor(() => expect(mockGet).toHaveBeenCalled());
    const select = await screen.findByRole('combobox', { name: /backend/i });
    expect(select).toHaveValue('auto');
  });

  it('changing backend persists the selection', async () => {
    renderWithProviders(<SandboxSettingsPanel />);
    const select = await screen.findByRole('combobox', { name: /backend/i });
    fireEvent.change(select, { target: { value: 'docker' } });
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(expect.objectContaining({ backend: 'docker' }))
    );
  });

  it('toggling the enabled switch persists the change', async () => {
    renderWithProviders(<SandboxSettingsPanel />);
    await waitFor(() => expect(mockGet).toHaveBeenCalled());
    const toggle = await screen.findByRole('switch', { name: /enable sandbox/i });
    fireEvent.click(toggle);
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(expect.objectContaining({ enabled: false }))
    );
  });

  it('renders Docker image input with current value', async () => {
    renderWithProviders(<SandboxSettingsPanel />);
    const input = await screen.findByRole('textbox', { name: /image/i });
    expect(input).toHaveValue('alpine:3.20');
  });

  it('blurring the Docker image input persists the value', async () => {
    renderWithProviders(<SandboxSettingsPanel />);
    const input = await screen.findByRole('textbox', { name: /image/i });
    fireEvent.change(input, { target: { value: 'node:20-slim' } });
    fireEvent.blur(input);
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(
        expect.objectContaining({ docker_image: 'node:20-slim' })
      )
    );
  });

  it('renders env passthrough variables as tags', async () => {
    renderWithProviders(<SandboxSettingsPanel />);
    expect(await screen.findByText('PATH')).toBeInTheDocument();
    expect(screen.getByText('HOME')).toBeInTheDocument();
    expect(screen.getByText('TERM')).toBeInTheDocument();
  });

  it('shows desktop-only message when not in Tauri', async () => {
    vi.mocked(isTauri).mockReturnValue(false);
    renderWithProviders(<SandboxSettingsPanel />);
    expect(await screen.findByText(/sandbox settings are only available/i)).toBeInTheDocument();
    expect(mockGet).not.toHaveBeenCalled();
  });

  it('shows error when settings fail to load', async () => {
    mockGet.mockRejectedValue(new Error('RPC timeout'));
    renderWithProviders(<SandboxSettingsPanel />);
    expect(await screen.findByText('RPC timeout')).toBeInTheDocument();
  });

  it('shows saved note after successful persist', async () => {
    renderWithProviders(<SandboxSettingsPanel />);
    const select = await screen.findByRole('combobox', { name: /backend/i });
    fireEvent.change(select, { target: { value: 'none' } });
    expect(await screen.findByText(/applies to new agent sessions/i)).toBeInTheDocument();
  });

  it('shows error note when persist fails', async () => {
    mockUpdate.mockRejectedValue(new Error('Save failed'));
    renderWithProviders(<SandboxSettingsPanel />);
    const select = await screen.findByRole('combobox', { name: /backend/i });
    fireEvent.change(select, { target: { value: 'docker' } });
    expect(await screen.findByText('Save failed')).toBeInTheDocument();
  });

  // ─── Memory limit field ───────────────────────────────────────────────────

  it('renders memory limit input with current value', async () => {
    renderWithProviders(<SandboxSettingsPanel />);
    const input = await screen.findByRole('spinbutton', { name: /memory limit/i });
    expect(input).toHaveValue(512);
  });

  it('blurring the memory limit input persists the value', async () => {
    renderWithProviders(<SandboxSettingsPanel />);
    const input = await screen.findByRole('spinbutton', { name: /memory limit/i });
    fireEvent.change(input, { target: { value: '1024' } });
    fireEvent.blur(input);
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(
        expect.objectContaining({ docker_memory_limit_mb: 1024 })
      )
    );
  });

  it('pressing Enter in the memory limit input persists the value', async () => {
    renderWithProviders(<SandboxSettingsPanel />);
    const input = await screen.findByRole('spinbutton', { name: /memory limit/i });
    fireEvent.change(input, { target: { value: '256' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(
        expect.objectContaining({ docker_memory_limit_mb: 256 })
      )
    );
  });

  it('clearing the memory limit and blurring persists null', async () => {
    renderWithProviders(<SandboxSettingsPanel />);
    const input = await screen.findByRole('spinbutton', { name: /memory limit/i });
    fireEvent.change(input, { target: { value: '' } });
    fireEvent.blur(input);
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(
        expect.objectContaining({ docker_memory_limit_mb: null })
      )
    );
  });

  // ─── CPU limit field ──────────────────────────────────────────────────────

  it('renders CPU limit input with current value', async () => {
    renderWithProviders(<SandboxSettingsPanel />);
    const input = await screen.findByRole('spinbutton', { name: /cpu limit/i });
    expect(input).toHaveValue(1.0);
  });

  it('blurring the CPU limit input persists the value', async () => {
    renderWithProviders(<SandboxSettingsPanel />);
    const input = await screen.findByRole('spinbutton', { name: /cpu limit/i });
    fireEvent.change(input, { target: { value: '2' } });
    fireEvent.blur(input);
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(expect.objectContaining({ docker_cpu_limit: 2 }))
    );
  });

  it('pressing Enter in the CPU limit input persists the value', async () => {
    renderWithProviders(<SandboxSettingsPanel />);
    const input = await screen.findByRole('spinbutton', { name: /cpu limit/i });
    fireEvent.change(input, { target: { value: '0.5' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(expect.objectContaining({ docker_cpu_limit: 0.5 }))
    );
  });

  it('clearing the CPU limit and blurring persists null', async () => {
    renderWithProviders(<SandboxSettingsPanel />);
    const input = await screen.findByRole('spinbutton', { name: /cpu limit/i });
    fireEvent.change(input, { target: { value: '' } });
    fireEvent.blur(input);
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(expect.objectContaining({ docker_cpu_limit: null }))
    );
  });

  // ─── Empty env passthrough ────────────────────────────────────────────────

  it('shows empty state when no env passthrough variables are configured', async () => {
    mockGet.mockResolvedValue({ result: sandboxSettings({ env_passthrough: [] }), logs: [] });
    renderWithProviders(<SandboxSettingsPanel />);
    await waitFor(() => expect(mockGet).toHaveBeenCalled());
    // Empty state text appears when env_passthrough is empty
    expect(await screen.findByText(/no environment variables configured/i)).toBeInTheDocument();
  });
});
