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
  connections: [{ type: 'stdio', published: true }],
  required_env_keys: ['API_KEY', 'SECRET_TOKEN'],
};

const DETAIL_NO_ENV = {
  qualified_name: 'acme/simple-server',
  display_name: 'Simple Server',
  description: 'No env needed',
  connections: [{ type: 'stdio', published: true }],
  required_env_keys: [],
};

async function goToConfigureStep() {
  await waitFor(() => screen.getByRole('button', { name: 'Configure & install' }));
  fireEvent.click(screen.getByRole('button', { name: 'Configure & install' }));
}

describe('InstallDialog', () => {
  beforeEach(() => {
    mockRegistryGet.mockReset();
    mockInstall.mockReset();
    mockConnect.mockReset();
    mockConnect.mockResolvedValue({ server_id: 'srv-1', status: 'connected', tools: [] });
  });

  it('shows loading state while fetching detail', () => {
    mockRegistryGet.mockReturnValue(new Promise(() => {}));
    render(
      <InstallDialog qualifiedName="acme/test-server" onSuccess={() => {}} onCancel={() => {}} />
    );
    expect(screen.getByText('Loading server details...')).toBeInTheDocument();
  });

  it('renders detail overview with server info', async () => {
    mockRegistryGet.mockResolvedValue(DETAIL);
    render(
      <InstallDialog qualifiedName="acme/test-server" onSuccess={() => {}} onCancel={() => {}} />
    );

    await waitFor(() => {
      expect(screen.getByText('Test Server')).toBeInTheDocument();
    });
    expect(screen.getByText('A test server')).toBeInTheDocument();
    expect(screen.getByText('Requires configuration')).toBeInTheDocument();
    expect(screen.getByText('Runs locally')).toBeInTheDocument();
  });

  it('shows env key preview badges on detail step', async () => {
    mockRegistryGet.mockResolvedValue(DETAIL);
    render(
      <InstallDialog qualifiedName="acme/test-server" onSuccess={() => {}} onCancel={() => {}} />
    );

    await waitFor(() => screen.getByText('API_KEY'));
    expect(screen.getByText('SECRET_TOKEN')).toBeInTheDocument();
  });

  it('renders env key inputs after clicking configure', async () => {
    mockRegistryGet.mockResolvedValue(DETAIL);
    render(
      <InstallDialog qualifiedName="acme/test-server" onSuccess={() => {}} onCancel={() => {}} />
    );

    await goToConfigureStep();
    expect(screen.getByLabelText('API_KEY')).toBeInTheDocument();
    expect(screen.getByLabelText('SECRET_TOKEN')).toBeInTheDocument();
  });

  it('renders env inputs as password type by default', async () => {
    mockRegistryGet.mockResolvedValue(DETAIL);
    render(
      <InstallDialog qualifiedName="acme/test-server" onSuccess={() => {}} onCancel={() => {}} />
    );

    await goToConfigureStep();
    const input = screen.getByLabelText('API_KEY') as HTMLInputElement;
    expect(input.type).toBe('password');
  });

  it('toggles env input to text on Show click', async () => {
    mockRegistryGet.mockResolvedValue(DETAIL);
    render(
      <InstallDialog qualifiedName="acme/test-server" onSuccess={() => {}} onCancel={() => {}} />
    );

    await goToConfigureStep();
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

    await goToConfigureStep();
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

    await goToConfigureStep();
    fireEvent.change(screen.getByLabelText('API_KEY'), { target: { value: 'k' } });
    fireEvent.change(screen.getByLabelText('SECRET_TOKEN'), { target: { value: 's' } });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Install' }));
    });

    expect(mockConnect).toHaveBeenCalledWith('srv-1');
    expect(onSuccess).toHaveBeenCalledWith(installedServer);
  });

  it('shows validation error when required field is empty', async () => {
    mockRegistryGet.mockResolvedValue(DETAIL);
    render(
      <InstallDialog qualifiedName="acme/test-server" onSuccess={() => {}} onCancel={() => {}} />
    );

    await goToConfigureStep();
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

    await goToConfigureStep();
    fireEvent.change(screen.getByLabelText('API_KEY'), { target: { value: 'key' } });
    fireEvent.change(screen.getByLabelText('SECRET_TOKEN'), { target: { value: 'secret' } });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Install' }));
    });

    await waitFor(() => screen.getByText('Server error'));
  });

  it('calls onCancel when Cancel is clicked on detail step', async () => {
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

    await goToConfigureStep();
    const input = screen.getByLabelText('API_KEY') as HTMLInputElement;
    expect(input.value).toBe('prefilled-key');
  });

  it('installs directly from detail step when no env keys required', async () => {
    const installedServer = {
      server_id: 'srv-2',
      ...DETAIL_NO_ENV,
      command_kind: 'node' as const,
      command: 'node',
      args: [],
      env_keys: [],
      installed_at: 2000,
    };
    mockRegistryGet.mockResolvedValue(DETAIL_NO_ENV);
    mockInstall.mockResolvedValue(installedServer);

    const onSuccess = vi.fn();
    render(
      <InstallDialog qualifiedName="acme/simple-server" onSuccess={onSuccess} onCancel={() => {}} />
    );

    await waitFor(() => screen.getByRole('button', { name: 'Install' }));
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Install' }));
    });

    expect(mockInstall).toHaveBeenCalledWith({
      qualified_name: 'acme/simple-server',
      env: {},
      config: undefined,
    });
    expect(onSuccess).toHaveBeenCalledWith(installedServer);
  });

  it('shows connection info on detail step', async () => {
    mockRegistryGet.mockResolvedValue({
      ...DETAIL,
      connections: [
        { type: 'stdio', published: true },
        { type: 'http', published: false, deployment_url: 'https://example.com/mcp' },
      ],
    });
    render(
      <InstallDialog qualifiedName="acme/test-server" onSuccess={() => {}} onCancel={() => {}} />
    );

    await waitFor(() => screen.getByText('Available connections'));
    expect(screen.getByText('stdio')).toBeInTheDocument();
    expect(screen.getByText('http')).toBeInTheDocument();
  });

  it('navigates back from configure to detail step', async () => {
    mockRegistryGet.mockResolvedValue(DETAIL);
    render(
      <InstallDialog qualifiedName="acme/test-server" onSuccess={() => {}} onCancel={() => {}} />
    );

    await goToConfigureStep();
    expect(screen.getByLabelText('API_KEY')).toBeInTheDocument();

    fireEvent.click(screen.getByText(`← Test Server`));
    await waitFor(() => {
      expect(screen.getByText('A test server')).toBeInTheDocument();
    });
  });
});
