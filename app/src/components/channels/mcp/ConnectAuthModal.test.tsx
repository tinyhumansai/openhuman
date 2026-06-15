import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import ConnectAuthModal from './ConnectAuthModal';

const mockConnect = vi.fn();
const mockUpdateEnv = vi.fn();
const mockDetectAuth = vi.fn();
const mockRegistryGet = vi.fn();
const mockOauthBegin = vi.fn();
const mockStatus = vi.fn();
const mockOpenUrl = vi.fn();

vi.mock('../../../services/api/mcpClientsApi', () => ({
  mcpClientsApi: {
    connect: (...args: unknown[]) => mockConnect(...args),
    updateEnv: (...args: unknown[]) => mockUpdateEnv(...args),
    detectAuth: (...args: unknown[]) => mockDetectAuth(...args),
    registryGet: (...args: unknown[]) => mockRegistryGet(...args),
    oauthBegin: (...args: unknown[]) => mockOauthBegin(...args),
    status: (...args: unknown[]) => mockStatus(...args),
    configAssist: vi.fn(),
  },
}));

vi.mock('../../../utils/openUrl', () => ({
  openUrl: (...args: unknown[]) => mockOpenUrl(...args),
}));

const BASE_SERVER = {
  server_id: 'srv-1',
  qualified_name: 'acme/test-server',
  display_name: 'Test Server',
  description: 'A test MCP server',
  // HTTP-remote installs still carry a CommandKind ('node'|'python'|'binary');
  // the transport, not command_kind, is what marks them remote. Use 'node' to
  // match the type (there is no 'http' CommandKind) and the sibling mocks.
  command_kind: 'node' as const,
  command: '',
  args: [],
  env_keys: ['Authorization'],
  installed_at: 1_700_000_000,
  enabled: true,
};

// A server with no declared env keys exercises the auto-seeded custom-header row.
const NO_KEYS_SERVER = { ...BASE_SERVER, env_keys: [] as string[] };

describe('ConnectAuthModal', () => {
  beforeEach(() => {
    mockConnect.mockReset();
    mockUpdateEnv.mockReset();
    mockDetectAuth.mockReset();
    mockRegistryGet.mockReset();
    mockOauthBegin.mockReset();
    mockStatus.mockReset();
    mockOpenUrl.mockReset();
    // Benign defaults: no special auth, no extra declared keys.
    mockDetectAuth.mockResolvedValue({ kind: 'token', grant_types: [] });
    mockRegistryGet.mockResolvedValue({
      qualified_name: 'acme/test-server',
      display_name: 'Test Server',
      connections: [],
      required_env_keys: [],
    });
  });

  it('renders the dialog title and an input for each declared key', async () => {
    render(<ConnectAuthModal server={BASE_SERVER} onClose={() => {}} onConnected={() => {}} />);
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
    expect(screen.getByText('Connect Test Server')).toBeInTheDocument();
    // Declared key from env_keys gets a labelled secret input.
    expect(screen.getByLabelText('Authorization')).toBeInTheDocument();
  });

  it('merges registry-declared required keys into the field list', async () => {
    mockRegistryGet.mockResolvedValue({
      qualified_name: 'acme/test-server',
      display_name: 'Test Server',
      connections: [],
      required_env_keys: ['X-API-Key'],
    });
    render(<ConnectAuthModal server={BASE_SERVER} onClose={() => {}} onConnected={() => {}} />);
    // Field appears once the best-effort registry_get resolves.
    expect(await screen.findByLabelText('X-API-Key')).toBeInTheDocument();
    // Original install key is still present.
    expect(screen.getByLabelText('Authorization')).toBeInTheDocument();
  });

  it('toggles secret visibility on Show/Hide', async () => {
    render(<ConnectAuthModal server={BASE_SERVER} onClose={() => {}} onConnected={() => {}} />);
    const input = (await screen.findByLabelText('Authorization')) as HTMLInputElement;
    expect(input.type).toBe('password');
    fireEvent.click(screen.getAllByRole('button', { name: 'Show' })[0]);
    expect(input.type).toBe('text');
  });

  it('renders a Bearer/None scheme select for declared keys', async () => {
    render(<ConnectAuthModal server={BASE_SERVER} onClose={() => {}} onConnected={() => {}} />);
    await screen.findByLabelText('Authorization');
    // Authorization defaults to the Bearer scheme.
    const selects = screen.getAllByRole('combobox');
    expect(selects.length).toBeGreaterThan(0);
    expect((selects[0] as HTMLSelectElement).value).toBe('bearer');
    fireEvent.change(selects[0], { target: { value: 'raw' } });
    expect((selects[0] as HTMLSelectElement).value).toBe('raw');
  });

  it('seeds a custom-header row when the server declares no keys', async () => {
    render(<ConnectAuthModal server={NO_KEYS_SERVER} onClose={() => {}} onConnected={() => {}} />);
    await screen.findByRole('dialog');
    // Auto-seeded row pre-fills the header name "Authorization".
    await waitFor(() => {
      expect(screen.getByDisplayValue('Authorization')).toBeInTheDocument();
    });
  });

  it('adds and removes custom header rows', async () => {
    render(<ConnectAuthModal server={BASE_SERVER} onClose={() => {}} onConnected={() => {}} />);
    await screen.findByRole('dialog');
    fireEvent.click(screen.getByRole('button', { name: '+ Add header' }));
    expect(screen.getByPlaceholderText('Header name')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Remove header' }));
    await waitFor(() => {
      expect(screen.queryByPlaceholderText('Header name')).not.toBeInTheDocument();
    });
  });

  it('connects with no auth supplied (plain connect path) and fires onConnected', async () => {
    const tools = [{ name: 'read_file', description: 'reads', input_schema: {} }];
    mockConnect.mockResolvedValue({ server_id: 'srv-1', status: 'connected', tools });
    const onConnected = vi.fn();
    const onClose = vi.fn();
    render(<ConnectAuthModal server={BASE_SERVER} onClose={onClose} onConnected={onConnected} />);
    const dialog = await screen.findByRole('dialog');
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Connect' }));
    });
    await waitFor(() => {
      expect(mockConnect).toHaveBeenCalledWith('srv-1');
      expect(onConnected).toHaveBeenCalledWith(tools);
      expect(onClose).toHaveBeenCalled();
    });
    expect(mockUpdateEnv).not.toHaveBeenCalled();
  });

  it('persists supplied credentials via update_env (with Bearer scheme applied)', async () => {
    const tools = [{ name: 't', input_schema: {} }];
    mockUpdateEnv.mockResolvedValue({
      server_id: 'srv-1',
      status: 'connected',
      env_keys: ['Authorization'],
      tools,
    });
    const onConnected = vi.fn();
    render(<ConnectAuthModal server={BASE_SERVER} onClose={() => {}} onConnected={onConnected} />);
    const input = await screen.findByLabelText('Authorization');
    fireEvent.change(input, { target: { value: 'tok-123' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
    });
    await waitFor(() => {
      expect(mockUpdateEnv).toHaveBeenCalledWith({
        server_id: 'srv-1',
        // Bearer scheme prepends "Bearer " to the Authorization value.
        env: { Authorization: 'Bearer tok-123' },
      });
      expect(onConnected).toHaveBeenCalledWith(tools);
    });
    expect(mockConnect).not.toHaveBeenCalled();
  });

  it('shows an error when update_env reconnect does not reach connected', async () => {
    mockUpdateEnv.mockResolvedValue({
      server_id: 'srv-1',
      status: 'disconnected',
      env_keys: ['Authorization'],
      error: 'bad token',
    });
    render(<ConnectAuthModal server={BASE_SERVER} onClose={() => {}} onConnected={() => {}} />);
    const input = await screen.findByLabelText('Authorization');
    fireEvent.change(input, { target: { value: 'tok' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
    });
    await waitFor(() => {
      expect(screen.getByText('bad token')).toBeInTheDocument();
    });
  });

  it('shows an error when connect rejects', async () => {
    mockConnect.mockRejectedValue(new Error('Connection refused'));
    render(<ConnectAuthModal server={BASE_SERVER} onClose={() => {}} onConnected={() => {}} />);
    const dialog = await screen.findByRole('dialog');
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Connect' }));
    });
    await waitFor(() => {
      expect(screen.getByText('Connection refused')).toBeInTheDocument();
    });
  });

  it('shows the OAuth sign-in section when detectAuth reports oauth', async () => {
    mockDetectAuth.mockResolvedValue({
      kind: 'oauth',
      authorization_endpoint: 'https://auth.example/authorize',
      grant_types: ['authorization_code'],
    });
    render(<ConnectAuthModal server={BASE_SERVER} onClose={() => {}} onConnected={() => {}} />);
    expect(await screen.findByRole('button', { name: 'Sign in with browser' })).toBeInTheDocument();
  });

  it('OAuth sign-in opens the authorize URL and connects once status flips', async () => {
    mockDetectAuth.mockResolvedValue({ kind: 'oauth', grant_types: ['authorization_code'] });
    mockOauthBegin.mockResolvedValue('https://auth.example/authorize?x=1');
    mockOpenUrl.mockResolvedValue(undefined);
    // First status poll already reports connected, so we skip the timer path.
    mockStatus.mockResolvedValue([{ server_id: 'srv-1', status: 'connected', tool_count: 1 }]);
    const tools = [{ name: 'read_file', input_schema: {} }];
    mockConnect.mockResolvedValue({ server_id: 'srv-1', status: 'connected', tools });
    const onConnected = vi.fn();
    const onClose = vi.fn();
    render(<ConnectAuthModal server={BASE_SERVER} onClose={onClose} onConnected={onConnected} />);
    const signIn = await screen.findByRole('button', { name: 'Sign in with browser' });
    await act(async () => {
      fireEvent.click(signIn);
    });
    await waitFor(() => {
      expect(mockOauthBegin).toHaveBeenCalledWith('srv-1');
      expect(mockOpenUrl).toHaveBeenCalledWith('https://auth.example/authorize?x=1');
      expect(mockConnect).toHaveBeenCalledWith('srv-1');
      expect(onConnected).toHaveBeenCalledWith(tools);
      expect(onClose).toHaveBeenCalled();
    });
  });

  it('surfaces an OAuth begin failure as an error', async () => {
    mockDetectAuth.mockResolvedValue({ kind: 'oauth', grant_types: ['authorization_code'] });
    mockOauthBegin.mockRejectedValue(new Error('discovery failed'));
    render(<ConnectAuthModal server={BASE_SERVER} onClose={() => {}} onConnected={() => {}} />);
    const signIn = await screen.findByRole('button', { name: 'Sign in with browser' });
    await act(async () => {
      fireEvent.click(signIn);
    });
    await waitFor(() => {
      expect(screen.getByText('discovery failed')).toBeInTheDocument();
    });
  });

  it('closes via the Cancel button', async () => {
    const onClose = vi.fn();
    render(<ConnectAuthModal server={BASE_SERVER} onClose={onClose} onConnected={() => {}} />);
    await screen.findByRole('dialog');
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(onClose).toHaveBeenCalled();
  });

  it('closes on backdrop mousedown', async () => {
    const onClose = vi.fn();
    render(<ConnectAuthModal server={BASE_SERVER} onClose={onClose} onConnected={() => {}} />);
    const dialog = await screen.findByRole('dialog');
    fireEvent.mouseDown(dialog);
    expect(onClose).toHaveBeenCalled();
  });

  it('opens the config-help modal from the "Help & configure" link', async () => {
    mockDetectAuth.mockResolvedValue({ kind: 'none', grant_types: [] });
    render(<ConnectAuthModal server={BASE_SERVER} onClose={() => {}} onConnected={() => {}} />);
    await screen.findByRole('dialog');
    // The link in the modal header (not the stacked modal's heading) opens the
    // ConfigHelpModal, which renders its own dialog with the same label.
    fireEvent.click(screen.getByRole('button', { name: 'Help & configure' }));
    await waitFor(() => {
      // Two dialogs now: the connect modal + the stacked help modal.
      expect(screen.getAllByRole('dialog').length).toBeGreaterThan(1);
    });
  });

  it('falls back to token fields when detectAuth throws', async () => {
    mockDetectAuth.mockRejectedValue(new Error('probe failed'));
    render(<ConnectAuthModal server={BASE_SERVER} onClose={() => {}} onConnected={() => {}} />);
    // Non-fatal: the modal still renders and shows the declared key field.
    expect(await screen.findByLabelText('Authorization')).toBeInTheDocument();
  });
});
