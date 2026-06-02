import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import InstallDialog from './InstallDialog';

const mockRegistryGet = vi.fn();
const mockInstall = vi.fn();
const mockConnect = vi.fn();

vi.mock('../../../services/api/mcpClientsApi', () => ({
  mcpClientsApi: {
    registryGet: (...args: unknown[]) => mockRegistryGet(...args),
    install: (...args: unknown[]) => mockInstall(...args),
    connect: (...args: unknown[]) => mockConnect(...args),
  },
}));

const DETAIL = {
  qualified_name: 'acme/test-server',
  display_name: 'Test Server',
  description: 'A test server',
  connections: [],
  required_env_keys: ['API_KEY', 'SECRET_TOKEN'],
};

describe('InstallDialog', () => {
  beforeEach(() => {
    mockRegistryGet.mockReset();
    mockInstall.mockReset();
    mockConnect.mockReset();
    mockConnect.mockResolvedValue({ server_id: 'srv-1', status: 'connected', tools: [] });
  });

  it('shows loading state while fetching detail', () => {
    // Never resolves within the test
    mockRegistryGet.mockReturnValue(new Promise(() => {}));
    render(
      <InstallDialog qualifiedName="acme/test-server" onSuccess={() => {}} onCancel={() => {}} />
    );
    expect(screen.getByText('Loading server details...')).toBeInTheDocument();
  });

  it('renders env key inputs from registry_get', async () => {
    mockRegistryGet.mockResolvedValue(DETAIL);
    render(
      <InstallDialog qualifiedName="acme/test-server" onSuccess={() => {}} onCancel={() => {}} />
    );

    await waitFor(() => {
      expect(screen.getByLabelText('API_KEY')).toBeInTheDocument();
    });
    expect(screen.getByLabelText('SECRET_TOKEN')).toBeInTheDocument();
  });

  it('renders env inputs as password type by default', async () => {
    mockRegistryGet.mockResolvedValue(DETAIL);
    render(
      <InstallDialog qualifiedName="acme/test-server" onSuccess={() => {}} onCancel={() => {}} />
    );

    await waitFor(() => screen.getByLabelText('API_KEY'));

    const input = screen.getByLabelText('API_KEY') as HTMLInputElement;
    expect(input.type).toBe('password');
  });

  it('toggles env input to text on Show click', async () => {
    mockRegistryGet.mockResolvedValue(DETAIL);
    render(
      <InstallDialog qualifiedName="acme/test-server" onSuccess={() => {}} onCancel={() => {}} />
    );

    await waitFor(() => screen.getByLabelText('API_KEY'));

    const showButtons = screen.getAllByRole('button', { name: 'Show' });
    fireEvent.click(showButtons[0]);

    const input = screen.getByLabelText('API_KEY') as HTMLInputElement;
    expect(input.type).toBe('text');
  });

  it('calls install with filled values on submit', async () => {
    const installedServer = {
      server_id: 'srv-1',
      ...DETAIL,
      command_kind: 'node' as const,
      command: 'node',
      args: [],
      env_keys: ['API_KEY', 'SECRET_TOKEN'],
      installed_at: 1000,
    };
    mockRegistryGet.mockResolvedValue(DETAIL);
    mockInstall.mockResolvedValue(installedServer);

    const onSuccess = vi.fn();
    render(
      <InstallDialog qualifiedName="acme/test-server" onSuccess={onSuccess} onCancel={() => {}} />
    );

    await waitFor(() => screen.getByLabelText('API_KEY'));

    fireEvent.change(screen.getByLabelText('API_KEY'), { target: { value: 'my-api-key' } });
    fireEvent.change(screen.getByLabelText('SECRET_TOKEN'), { target: { value: 'my-secret' } });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Install' }));
    });

    expect(mockInstall).toHaveBeenCalledWith({
      qualified_name: 'acme/test-server',
      env: { API_KEY: 'my-api-key', SECRET_TOKEN: 'my-secret' },
      config: undefined,
    });
    // Auto-connect on success (issue #3039 gap B3).
    expect(mockConnect).toHaveBeenCalledWith('srv-1');
    expect(onSuccess).toHaveBeenCalledWith(installedServer);
  });

  it('still reports success when auto-connect fails (best-effort)', async () => {
    const installedServer = {
      server_id: 'srv-1',
      ...DETAIL,
      command_kind: 'node' as const,
      command: 'node',
      args: [],
      env_keys: ['API_KEY', 'SECRET_TOKEN'],
      installed_at: 1000,
    };
    mockRegistryGet.mockResolvedValue(DETAIL);
    mockInstall.mockResolvedValue(installedServer);
    mockConnect.mockRejectedValue(new Error('spawn failed'));

    const onSuccess = vi.fn();
    render(
      <InstallDialog qualifiedName="acme/test-server" onSuccess={onSuccess} onCancel={() => {}} />
    );

    await waitFor(() => screen.getByLabelText('API_KEY'));
    fireEvent.change(screen.getByLabelText('API_KEY'), { target: { value: 'k' } });
    fireEvent.change(screen.getByLabelText('SECRET_TOKEN'), { target: { value: 's' } });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Install' }));
    });

    expect(mockConnect).toHaveBeenCalledWith('srv-1');
    // A connect failure must NOT block the install success callback.
    expect(onSuccess).toHaveBeenCalledWith(installedServer);
  });

  it('shows validation error when required field is empty', async () => {
    mockRegistryGet.mockResolvedValue(DETAIL);
    render(
      <InstallDialog qualifiedName="acme/test-server" onSuccess={() => {}} onCancel={() => {}} />
    );

    await waitFor(() => screen.getByLabelText('API_KEY'));

    // Leave API_KEY empty, fill only SECRET_TOKEN
    fireEvent.change(screen.getByLabelText('SECRET_TOKEN'), { target: { value: 'secret' } });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Install' }));
    });

    expect(screen.getByText('"API_KEY" is required')).toBeInTheDocument();
    expect(mockInstall).not.toHaveBeenCalled();
  });

  it('shows install error on failure', async () => {
    mockRegistryGet.mockResolvedValue(DETAIL);
    mockInstall.mockRejectedValue(new Error('Server error'));

    render(
      <InstallDialog qualifiedName="acme/test-server" onSuccess={() => {}} onCancel={() => {}} />
    );

    await waitFor(() => screen.getByLabelText('API_KEY'));

    fireEvent.change(screen.getByLabelText('API_KEY'), { target: { value: 'key' } });
    fireEvent.change(screen.getByLabelText('SECRET_TOKEN'), { target: { value: 'secret' } });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Install' }));
    });

    await waitFor(() => screen.getByText('Server error'));
  });

  it('calls onCancel when Cancel is clicked', async () => {
    mockRegistryGet.mockResolvedValue(DETAIL);
    const onCancel = vi.fn();
    render(
      <InstallDialog qualifiedName="acme/test-server" onSuccess={() => {}} onCancel={onCancel} />
    );

    await waitFor(() => screen.getByRole('button', { name: 'Cancel' }));
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it('pre-fills env values from prefillEnv prop', async () => {
    mockRegistryGet.mockResolvedValue(DETAIL);
    render(
      <InstallDialog
        qualifiedName="acme/test-server"
        prefillEnv={{ API_KEY: 'prefilled-key' }}
        onSuccess={() => {}}
        onCancel={() => {}}
      />
    );

    await waitFor(() => screen.getByLabelText('API_KEY'));
    const input = screen.getByLabelText('API_KEY') as HTMLInputElement;
    expect(input.value).toBe('prefilled-key');
  });
});
