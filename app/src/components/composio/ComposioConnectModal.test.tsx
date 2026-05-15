import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { authorize } from '../../lib/composio/composioApi';
import { type ComposioConnection } from '../../lib/composio/types';
import { openUrl } from '../../utils/openUrl';
import ComposioConnectModal, {
  isMissingAtlassianSubdomainError,
  normalizeAtlassianSubdomain,
} from './ComposioConnectModal';
import { composioToolkitMeta } from './toolkitMeta';

vi.mock('../../lib/composio/composioApi', () => ({
  authorize: vi.fn(),
  deleteConnection: vi.fn(),
  getUserScopes: vi.fn(() => Promise.resolve({ read: true, write: true, admin: false })),
  listConnections: vi.fn(),
  setUserScopes: vi.fn(),
}));

vi.mock('../../utils/openUrl', () => ({ openUrl: vi.fn() }));

// Mock TriggerToggles because it does its own API calls
vi.mock('./TriggerToggles', () => ({ default: () => <div data-testid="trigger-toggles" /> }));

const mockToolkit = composioToolkitMeta('gmail');
const jiraToolkit = composioToolkitMeta('jira');

describe('<ComposioConnectModal>', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(authorize).mockResolvedValue({
      connectUrl: 'https://composio.example/jira/consent',
      connectionId: 'conn-123',
    });
    vi.mocked(openUrl).mockResolvedValue(undefined);
  });

  it('hides raw connection ID and "id:" label in connected phase', () => {
    const connection: ComposioConnection = { id: 'ca_xyz', toolkit: 'gmail', status: 'ACTIVE' };

    render(
      <ComposioConnectModal toolkit={mockToolkit} connection={connection} onClose={() => {}} />
    );

    // Should be in 'connected' phase because connection.status is 'ACTIVE'
    expect(screen.getByText(/Gmail is connected/)).toBeInTheDocument();
    expect(screen.queryByText(/ca_xyz/)).not.toBeInTheDocument();
    expect(screen.queryByText(/id:/)).not.toBeInTheDocument();
  });

  it('renders accountEmail when provided', () => {
    const connection: ComposioConnection = {
      id: 'ca_xyz',
      toolkit: 'gmail',
      status: 'ACTIVE',
      accountEmail: 'foo@bar.com',
    };

    render(
      <ComposioConnectModal toolkit={mockToolkit} connection={connection} onClose={() => {}} />
    );

    expect(screen.getByText('(foo@bar.com)')).toBeInTheDocument();
  });

  it('renders workspace when accountEmail is missing', () => {
    const connection: ComposioConnection = {
      id: 'ca_xyz',
      toolkit: 'gmail',
      status: 'ACTIVE',
      workspace: 'Acme',
    };

    render(
      <ComposioConnectModal toolkit={mockToolkit} connection={connection} onClose={() => {}} />
    );

    expect(screen.getByText('(Acme)')).toBeInTheDocument();
  });

  it('renders username when email and workspace are missing', () => {
    const connection: ComposioConnection = {
      id: 'ca_xyz',
      toolkit: 'gmail',
      status: 'ACTIVE',
      username: 'oxox',
    };

    render(
      <ComposioConnectModal toolkit={mockToolkit} connection={connection} onClose={() => {}} />
    );

    expect(screen.getByText('(oxox)')).toBeInTheDocument();
  });

  it('prioritizes accountEmail over workspace and username', () => {
    const connection: ComposioConnection = {
      id: 'ca_xyz',
      toolkit: 'gmail',
      status: 'ACTIVE',
      accountEmail: 'foo@bar.com',
      workspace: 'Acme',
      username: 'oxox',
    };

    render(
      <ComposioConnectModal toolkit={mockToolkit} connection={connection} onClose={() => {}} />
    );

    expect(screen.getByText('(foo@bar.com)')).toBeInTheDocument();
    expect(screen.queryByText('(Acme)')).not.toBeInTheDocument();
    expect(screen.queryByText('(oxox)')).not.toBeInTheDocument();
  });

  it('keeps default toolkit authorization free of empty extra params', async () => {
    render(
      <ComposioConnectModal toolkit={mockToolkit} connection={undefined} onClose={() => {}} />
    );

    fireEvent.click(screen.getByRole('button', { name: 'Connect Gmail' }));

    await waitFor(() => {
      expect(authorize).toHaveBeenCalledWith('gmail', undefined);
    });
  });

  it('normalizes pasted Atlassian URLs to the Jira subdomain', () => {
    expect(normalizeAtlassianSubdomain('https://Acme.atlassian.net/jira/software')).toBe('acme');
    expect(normalizeAtlassianSubdomain('acme.atlassian.net')).toBe('acme');
  });

  it('detects Composio missing-subdomain errors without exposing raw payloads', () => {
    expect(
      isMissingAtlassianSubdomainError(
        'Composio authorization failed: {"error":{"slug":"ConnectedAccount_MissingRequiredFields","message":"Missing required fields: Your Subdomain"}}'
      )
    ).toBe(true);
  });

  it('requires an Atlassian subdomain before Jira authorization', async () => {
    render(
      <ComposioConnectModal toolkit={jiraToolkit} connection={undefined} onClose={() => {}} />
    );

    fireEvent.click(screen.getByRole('button', { name: 'Connect Jira' }));

    expect(await screen.findByText(/Enter your Atlassian subdomain/i)).toBeInTheDocument();
    expect(authorize).not.toHaveBeenCalled();
    expect(openUrl).not.toHaveBeenCalled();
  });

  it('sends the normalized Jira subdomain as an authorize extra param', async () => {
    render(
      <ComposioConnectModal toolkit={jiraToolkit} connection={undefined} onClose={() => {}} />
    );

    fireEvent.change(screen.getByLabelText(/Atlassian subdomain/i), {
      target: { value: 'https://Acme.atlassian.net/jira/software' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Connect Jira' }));

    await waitFor(() => {
      expect(authorize).toHaveBeenCalledWith('jira', { subdomain: 'acme' });
    });
    expect(openUrl).toHaveBeenCalledWith('https://composio.example/jira/consent');
  });

  it('maps Jira missing-field backend errors back to the inline subdomain form', async () => {
    vi.mocked(authorize).mockRejectedValueOnce(
      new Error(
        'Composio authorization failed: 400 {"error":{"slug":"ConnectedAccount_MissingRequiredFields","message":"Missing required fields: Your Subdomain"}}'
      )
    );

    render(
      <ComposioConnectModal toolkit={jiraToolkit} connection={undefined} onClose={() => {}} />
    );

    fireEvent.change(screen.getByLabelText(/Atlassian subdomain/i), { target: { value: 'acme' } });
    fireEvent.click(screen.getByRole('button', { name: 'Connect Jira' }));

    expect(
      await screen.findByText(/Jira needs your Atlassian subdomain before authorization/i)
    ).toBeInTheDocument();
    expect(screen.getByLabelText(/Atlassian subdomain/i)).toBeInTheDocument();
    expect(screen.queryByText(/ConnectedAccount_MissingRequiredFields/i)).not.toBeInTheDocument();
    expect(openUrl).not.toHaveBeenCalled();
  });
});
