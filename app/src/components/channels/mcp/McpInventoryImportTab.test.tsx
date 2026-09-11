import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import McpInventoryImportTab from './McpInventoryImportTab';
import { buildManifest, serializeManifest } from './McpInventoryManifest';
import type { InstalledServer } from './types';

const SERVER: InstalledServer = {
  server_id: 'srv-1',
  qualified_name: 'acme/db-server',
  display_name: 'DB Server',
  command_kind: 'node',
  command: 'npx',
  args: ['-y', 'acme/db-server'],
  env_keys: ['DB_URL'],
  installed_at: 1_700_000_000,
  enabled: true,
};

describe('<McpInventoryImportTab />', () => {
  it('renders the paste textarea, disables Preview until there is input', () => {
    renderWithProviders(<McpInventoryImportTab installedServers={[]} onInstallServer={vi.fn()} />);
    expect(screen.getByLabelText('Paste manifest JSON')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Preview' })).toBeDisabled();
  });

  it('previews a pasted manifest and installs a new entry', () => {
    const onInstallServer = vi.fn();
    renderWithProviders(
      <McpInventoryImportTab installedServers={[]} onInstallServer={onInstallServer} />
    );
    const manifestText = serializeManifest(buildManifest([SERVER]));
    fireEvent.change(screen.getByLabelText('Paste manifest JSON'), {
      target: { value: manifestText },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Preview' }));
    expect(screen.getByRole('heading', { name: 'Preview' })).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Install DB Server from this manifest' }));
    expect(onInstallServer).toHaveBeenCalledWith('acme/db-server', { DB_URL: '' });
  });
});
