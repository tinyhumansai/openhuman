import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { type ComposioConnection } from '../../lib/composio/types';
import ComposioConnectModal from './ComposioConnectModal';
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

describe('<ComposioConnectModal>', () => {
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
});
