import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { useState } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import McpRegistryBrowser, { deriveRepoUrl, serverPageUrl } from './McpRegistryBrowser';
import type { InstalledServer } from './types';

const mockRegistrySearch = vi.fn();
const mockRegistryGet = vi.fn();
const mockConfigGet = vi.fn();
const mockConfigSet = vi.fn();
const mockInstalledList = vi.fn();
const mockDetectAuth = vi.fn();
const mockConnect = vi.fn();
const mockOpenUrl = vi.fn();

vi.mock('../../../services/api/mcpClientsApi', () => ({
  mcpClientsApi: {
    registrySearch: (...args: unknown[]) => mockRegistrySearch(...args),
    registryGet: (...args: unknown[]) => mockRegistryGet(...args),
    configGet: (...args: unknown[]) => mockConfigGet(...args),
    configSet: (...args: unknown[]) => mockConfigSet(...args),
    installedList: (...args: unknown[]) => mockInstalledList(...args),
    detectAuth: (...args: unknown[]) => mockDetectAuth(...args),
    connect: (...args: unknown[]) => mockConnect(...args),
  },
}));

vi.mock('../../../utils/openUrl', () => ({
  openUrl: (...args: unknown[]) => mockOpenUrl(...args),
}));

vi.mock('./ConnectAuthModal', () => ({
  default: ({
    server,
    onClose,
    onConnected,
  }: {
    server: InstalledServer;
    onClose: () => void;
    onConnected: (tools: unknown[]) => void;
  }) => (
    <div role="dialog" aria-label={`Sign in to ${server.display_name}`}>
      <button type="button" onClick={() => onConnected([])}>
        stub-connected
      </button>
      <button type="button" onClick={onClose}>
        stub-close
      </button>
    </div>
  ),
}));

const SERVERS = [
  {
    qualified_name: 'io.github.acme/echo',
    display_name: 'Echo',
    description: 'Echoes things.',
    source: 'mcp_official',
    official: true,
  },
  {
    qualified_name: 'vendor/hosted-thing',
    display_name: 'Hosted Thing',
    is_deployed: true,
    website_url: 'https://hosted.example',
    source: 'mcp_official',
  },
  {
    qualified_name: 'vendor/keyed',
    display_name: 'Keyed',
    is_deployed: true,
    source: 'mcp_official',
    auth_kind: 'api_key',
  },
  {
    qualified_name: 'smithery/hosted',
    display_name: 'Smithery Hosted',
    is_deployed: true,
    source: 'smithery',
  },
  { qualified_name: 'installed/already', display_name: 'Already Declared', source: 'smithery' },
];

const HOSTED_NAME = 'vendor/hosted-thing';

const installedRow = (qualified_name: string): InstalledServer => ({
  server_id: `srv-${qualified_name}`,
  qualified_name,
  display_name: qualified_name,
  command_kind: 'node',
  command: '',
  args: [],
  env_keys: [],
  installed_at: 1,
  transport: { kind: 'http_remote', url: 'https://hosted.example/mcp' },
  enabled: true,
});

const hostedDetail = (deployment_url = 'https://hosted.example/mcp') => ({
  ...SERVERS[1],
  connections: [{ type: 'http', deployment_url }],
  required_env_keys: [],
});

const onDeclaredSpy = vi.fn();

/** Mirrors the tab: re-reading the installed list marks the row declared. */
const Harness = ({ initial = ['installed/already'] }: { initial?: string[] }) => {
  const [names, setNames] = useState<ReadonlySet<string>>(() => new Set(initial));
  return (
    <McpRegistryBrowser
      installedNames={names}
      onDeclared={async () => {
        onDeclaredSpy();
        const rows = (await mockInstalledList()) as InstalledServer[];
        setNames(new Set(rows.map(r => r.qualified_name)));
      }}
    />
  );
};

const rowFor = (name: string) =>
  screen
    .getAllByTestId('mcp-registry-row')
    .find(row => within(row).queryByText(name, { exact: true })) as HTMLElement;

describe('serverPageUrl', () => {
  it('prefers the declared website, then the repository, then the directory listing', () => {
    expect(serverPageUrl(SERVERS[1])).toBe('https://hosted.example');
    expect(serverPageUrl(SERVERS[0])).toBe('https://github.com/acme/echo');
    expect(serverPageUrl(SERVERS[4])).toBe('https://smithery.ai/server/installed/already');
    expect(serverPageUrl({ qualified_name: 'com.vendor/x', display_name: 'X' })).toBe(
      'https://registry.modelcontextprotocol.io/?search=com.vendor%2Fx'
    );
  });

  it('derives repository urls only from code-host slugs', () => {
    expect(deriveRepoUrl('io.gitlab.acme/echo')).toBe('https://gitlab.com/acme/echo');
    expect(deriveRepoUrl('com.vendor/x')).toBeNull();
    expect(deriveRepoUrl('noslash')).toBeNull();
  });
});

describe('McpRegistryBrowser', () => {
  beforeEach(() => {
    for (const mock of [
      mockRegistrySearch,
      mockRegistryGet,
      mockConfigGet,
      mockConfigSet,
      mockInstalledList,
      mockDetectAuth,
      mockConnect,
      mockOpenUrl,
      onDeclaredSpy,
    ]) {
      mock.mockReset();
    }
    mockOpenUrl.mockResolvedValue(undefined);
    mockRegistrySearch.mockResolvedValue({ servers: SERVERS, page: 1, total_pages: 1 });
    mockRegistryGet.mockResolvedValue(hostedDetail());
    mockConfigGet.mockResolvedValue({ mcpServers: { 'installed/already': { command: 'npx' } } });
    mockConfigSet.mockResolvedValue({ mcpServers: {}, added: [], updated: [], removed: [] });
    mockInstalledList.mockResolvedValue([
      installedRow('installed/already'),
      installedRow(HOSTED_NAME),
    ]);
    mockDetectAuth.mockResolvedValue({ kind: 'none', grant_types: [] });
    mockConnect.mockResolvedValue({
      server_id: `srv-${HOSTED_NAME}`,
      status: 'connected',
      tools: [],
    });
  });

  it('offers Add for open hosted rows, Added for declared ones and Open page for the rest', async () => {
    render(<Harness />);
    const rows = await screen.findAllByTestId('mcp-registry-row');
    expect(rows).toHaveLength(5);

    expect(
      within(rowFor('Hosted Thing')).getByRole('button', { name: 'Add Hosted Thing' })
    ).toBeEnabled();
    expect(
      within(rowFor('Already Declared')).getByRole('button', { name: /Added/ })
    ).toBeDisabled();
    for (const name of ['Echo', 'Keyed', 'Smithery Hosted']) {
      expect(
        within(rowFor(name)).getByRole('button', { name: `Open the page for ${name}` })
      ).toBeInTheDocument();
    }

    fireEvent.click(screen.getByRole('button', { name: 'Open the page for Echo' }));
    expect(mockOpenUrl).toHaveBeenCalledWith('https://github.com/acme/echo');
  });

  it('declares an open hosted server, re-reads the rows, connects it and shows Added', async () => {
    render(<Harness />);
    await screen.findAllByTestId('mcp-registry-row');
    fireEvent.click(screen.getByRole('button', { name: 'Add Hosted Thing' }));

    await waitFor(() => expect(mockConnect).toHaveBeenCalledWith(`srv-${HOSTED_NAME}`));
    expect(mockRegistryGet).toHaveBeenCalledWith(HOSTED_NAME);
    expect(mockConfigSet).toHaveBeenCalledWith({
      mcpServers: {
        'installed/already': { command: 'npx' },
        [HOSTED_NAME]: { url: 'https://hosted.example/mcp' },
      },
    });
    expect(onDeclaredSpy).toHaveBeenCalledTimes(1);
    expect(mockDetectAuth).toHaveBeenCalledWith(`srv-${HOSTED_NAME}`);
    await waitFor(() =>
      expect(within(rowFor('Hosted Thing')).getByRole('button', { name: /Added/ })).toBeDisabled()
    );
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('opens the sign-in dialog when the server asks for OAuth', async () => {
    mockDetectAuth.mockResolvedValue({ kind: 'oauth', grant_types: ['authorization_code'] });
    render(<Harness />);
    await screen.findAllByTestId('mcp-registry-row');
    fireEvent.click(screen.getByRole('button', { name: 'Add Hosted Thing' }));

    expect(
      await screen.findByRole('dialog', { name: `Sign in to ${HOSTED_NAME}` })
    ).toBeInTheDocument();
    expect(mockConnect).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'stub-connected' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(onDeclaredSpy).toHaveBeenCalledTimes(2);
  });

  it('opens the sign-in dialog when the auth probe fails', async () => {
    mockDetectAuth.mockRejectedValue(new Error('probe failed'));
    render(<Harness />);
    await screen.findAllByTestId('mcp-registry-row');
    fireEvent.click(screen.getByRole('button', { name: 'Add Hosted Thing' }));

    await screen.findByRole('dialog');
    fireEvent.click(screen.getByRole('button', { name: 'stub-close' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(mockConnect).not.toHaveBeenCalled();
  });

  it('shows a refused write inline, and Retry repeats only the write', async () => {
    mockConfigSet.mockRejectedValueOnce(new Error('mcp.json refused the entry'));
    render(<Harness />);
    await screen.findAllByTestId('mcp-registry-row');
    fireEvent.click(screen.getByRole('button', { name: 'Add Hosted Thing' }));

    const error = await screen.findByTestId('mcp-registry-row-error');
    expect(error).toHaveTextContent("Couldn't add this server.");
    expect(error).toHaveTextContent('mcp.json refused the entry');
    expect(onDeclaredSpy).not.toHaveBeenCalled();

    fireEvent.click(within(error).getByRole('button', { name: 'Try again' }));
    await waitFor(() => expect(mockConnect).toHaveBeenCalledTimes(1));
    expect(mockRegistryGet).toHaveBeenCalledTimes(1);
    expect(mockConfigSet).toHaveBeenCalledTimes(2);
    await waitFor(() =>
      expect(screen.queryByTestId('mcp-registry-row-error')).not.toBeInTheDocument()
    );
  });

  it('shows a failed connect inline, and Retry reconnects without rewriting', async () => {
    mockConnect.mockRejectedValueOnce(new Error('connection refused'));
    render(<Harness />);
    await screen.findAllByTestId('mcp-registry-row');
    fireEvent.click(screen.getByRole('button', { name: 'Add Hosted Thing' }));

    const error = await screen.findByTestId('mcp-registry-row-error');
    expect(error).toHaveTextContent("Added, but the server didn't connect.");
    expect(error).toHaveTextContent('connection refused');
    expect(within(rowFor('Hosted Thing')).getByRole('button', { name: /Added/ })).toBeDisabled();

    fireEvent.click(within(error).getByRole('button', { name: 'Try again' }));
    await waitFor(() => expect(mockConnect).toHaveBeenCalledTimes(2));
    expect(mockConfigSet).toHaveBeenCalledTimes(1);
    expect(mockInstalledList).toHaveBeenCalledTimes(2);
    await waitFor(() =>
      expect(screen.queryByTestId('mcp-registry-row-error')).not.toBeInTheDocument()
    );
  });

  it('writes nothing for a server that needs setup and falls back to its page', async () => {
    mockRegistryGet.mockResolvedValue(hostedDetail('https://{tenant}.hosted.example/mcp'));
    render(<Harness />);
    await screen.findAllByTestId('mcp-registry-row');
    fireEvent.click(screen.getByRole('button', { name: 'Add Hosted Thing' }));

    expect(await screen.findByTestId('mcp-registry-needs-setup')).toBeInTheDocument();
    expect(mockConfigGet).not.toHaveBeenCalled();
    expect(mockConfigSet).not.toHaveBeenCalled();
    expect(onDeclaredSpy).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Open the page for Hosted Thing' }));
    expect(mockOpenUrl).toHaveBeenCalledWith('https://hosted.example');
  });

  it('reports a failed detail lookup with a Retry that starts over', async () => {
    mockRegistryGet.mockRejectedValueOnce(new Error('registry down'));
    render(<Harness />);
    await screen.findAllByTestId('mcp-registry-row');
    fireEvent.click(screen.getByRole('button', { name: 'Add Hosted Thing' }));

    const error = await screen.findByTestId('mcp-registry-row-error');
    expect(error).toHaveTextContent('registry down');
    fireEvent.click(within(error).getByRole('button', { name: 'Try again' }));
    await waitFor(() => expect(mockConnect).toHaveBeenCalledTimes(1));
    expect(mockRegistryGet).toHaveBeenCalledTimes(2);
  });

  it('reports a declared server that the installed list does not return', async () => {
    mockInstalledList.mockResolvedValue([installedRow('installed/already')]);
    render(<Harness />);
    await screen.findAllByTestId('mcp-registry-row');
    fireEvent.click(screen.getByRole('button', { name: 'Add Hosted Thing' }));

    const error = await screen.findByTestId('mcp-registry-row-error');
    expect(error).toHaveTextContent("Couldn't add this server.");
    expect(mockDetectAuth).not.toHaveBeenCalled();
  });

  it('lists declared rows instead of hiding them', async () => {
    mockRegistrySearch.mockResolvedValue({ servers: [SERVERS[4]], page: 1, total_pages: 1 });
    render(<Harness />);
    await screen.findAllByTestId('mcp-registry-row');
    expect(screen.queryByTestId('mcp-catalog-empty')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Added/ })).toBeDisabled();
  });

  it('a nested link opens its own target only, not the row page as well', async () => {
    render(<Harness initial={[]} />);
    await screen.findAllByTestId('mcp-registry-row');
    fireEvent.click(screen.getByRole('button', { name: 'Repository' }));
    expect(mockOpenUrl).toHaveBeenCalledTimes(1);
    expect(mockOpenUrl).toHaveBeenCalledWith('https://github.com/acme/echo');
  });

  it('shows a directory outage with retry instead of an empty result', async () => {
    mockRegistrySearch.mockRejectedValueOnce(new Error('registry down'));
    render(<Harness initial={[]} />);
    await screen.findByTestId('mcp-catalog-error');
    expect(screen.queryByTestId('mcp-catalog-empty')).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
    await waitFor(() => expect(screen.getAllByTestId('mcp-registry-row')).toHaveLength(5));
  });

  it('re-queries the directory with the transport filter', async () => {
    render(<Harness initial={[]} />);
    await screen.findAllByTestId('mcp-registry-row');
    fireEvent.click(screen.getByRole('button', { name: 'Hosted' }));
    await waitFor(() =>
      expect(mockRegistrySearch).toHaveBeenLastCalledWith(
        expect.objectContaining({ transport: 'hosted', page: 1 })
      )
    );
  });
});
