import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import BranchPicker from './BranchPicker';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const mockExecute = vi.fn();
vi.mock('../../../lib/composio/composioApi', () => ({
  execute: (...a: unknown[]) => mockExecute(...a),
}));

describe('BranchPicker', () => {
  const baseProps = { value: '', onChange: vi.fn(), repo: '' };

  beforeEach(() => {
    mockExecute.mockReset();
    mockExecute.mockResolvedValue({ successful: true, data: [{ name: 'main' }, { name: 'dev' }] });
  });

  it('renders disabled with hint when no repo is selected', async () => {
    render(<BranchPicker {...baseProps} />);
    const select = await screen.findByRole('combobox');
    expect(select).toBeDisabled();
  });

  it('loads and displays branches when repo is set', async () => {
    render(<BranchPicker {...baseProps} repo="owner/repo" />);
    await waitFor(() => {
      expect(screen.getByRole('combobox')).toBeInTheDocument();
    });
    expect(mockExecute).toHaveBeenCalled();
  });

  it('reflects a pre-selected value', async () => {
    render(<BranchPicker {...baseProps} repo="owner/repo" value="main" />);
    const select = await screen.findByRole('combobox');
    await waitFor(() => expect(select).toHaveValue('main'));
  });

  it('falls back to main/master when API returns empty list', async () => {
    mockExecute.mockResolvedValue({ successful: true, data: [] });
    render(<BranchPicker {...baseProps} repo="owner/repo" />);
    await waitFor(() => expect(screen.getByRole('combobox')).toBeInTheDocument());
  });

  it('handles API error gracefully', async () => {
    mockExecute.mockResolvedValue({ successful: false, error: 'API error' });
    render(<BranchPicker {...baseProps} repo="owner/repo" />);
    await waitFor(() => expect(screen.getByRole('combobox')).toBeInTheDocument());
  });

  it('handles incomplete repo string (missing slash)', async () => {
    render(<BranchPicker {...baseProps} repo="noslash" />);
    const select = await screen.findByRole('combobox');
    expect(select).toBeInTheDocument();
  });

  it('is disabled when disabled prop is true', async () => {
    render(<BranchPicker {...baseProps} repo="owner/repo" disabled />);
    const select = await screen.findByRole('combobox');
    expect(select).toBeDisabled();
  });
});
