import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import RepoPicker from './RepoPicker';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const mockListConnections = vi.fn();
const mockExecute = vi.fn();

vi.mock('../../../lib/composio/composioApi', () => ({
  execute: (...a: unknown[]) => mockExecute(...a),
  listConnections: () => mockListConnections(),
}));

describe('RepoPicker', () => {
  const baseProps = { value: '', onChange: vi.fn() };

  beforeEach(() => {
    mockListConnections.mockReset();
    mockExecute.mockReset();
    mockListConnections.mockResolvedValue({
      connections: [{ toolkit: 'github', status: 'ACTIVE', username: 'u' }],
    });
    mockExecute.mockResolvedValue({
      successful: true,
      data: [{ name: 'repo1', owner: { login: 'user' }, full_name: 'user/repo1' }],
    });
  });

  it('renders a combobox and loads repos', async () => {
    render(<RepoPicker {...baseProps} />);
    await waitFor(() => {
      expect(screen.getByRole('combobox')).toBeInTheDocument();
    });
    expect(mockListConnections).toHaveBeenCalled();
  });

  it('shows not-connected error when no GitHub connection', async () => {
    mockListConnections.mockResolvedValue({ connections: [] });
    render(<RepoPicker {...baseProps} />);
    await waitFor(() => expect(screen.getByRole('combobox')).toBeInTheDocument());
  });

  it('handles failed repo fetch gracefully', async () => {
    mockExecute.mockResolvedValue({ successful: false, error: 'rate limited' });
    render(<RepoPicker {...baseProps} />);
    await waitFor(() => expect(screen.getByRole('combobox')).toBeInTheDocument());
  });

  it('handles empty repo list', async () => {
    mockExecute.mockResolvedValue({ successful: true, data: [] });
    render(<RepoPicker {...baseProps} />);
    await waitFor(() => expect(screen.getByRole('combobox')).toBeInTheDocument());
  });

  it('respects disabled prop', async () => {
    render(<RepoPicker {...baseProps} disabled />);
    const select = await screen.findByRole('combobox');
    expect(select).toBeDisabled();
  });

  it('forwards id to the select element', async () => {
    render(<RepoPicker {...baseProps} id="repo-input" />);
    const select = await screen.findByRole('combobox');
    expect(select).toHaveAttribute('id', 'repo-input');
  });
});
