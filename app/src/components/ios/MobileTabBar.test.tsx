import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import MobileTabBar from './MobileTabBar';

const navigate = vi.fn();
vi.mock('react-router-dom', async () => {
  const actual = await vi.importActual<typeof import('react-router-dom')>('react-router-dom');
  return { ...actual, useNavigate: () => navigate };
});

const renderAt = (path: string) =>
  render(
    <MemoryRouter initialEntries={[path]}>
      <MobileTabBar />
    </MemoryRouter>
  );

describe('MobileTabBar', () => {
  beforeEach(() => navigate.mockReset());

  it('renders Human, Chat, Settings tabs', () => {
    renderAt('/human');
    expect(screen.getByRole('button', { name: 'Human' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Chat' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Settings' })).toBeInTheDocument();
  });

  it('marks the active tab with aria-current=page', () => {
    renderAt('/chat');
    expect(screen.getByRole('button', { name: 'Chat' })).toHaveAttribute('aria-current', 'page');
    expect(screen.getByRole('button', { name: 'Human' })).not.toHaveAttribute('aria-current');
  });

  it('treats a deeper /settings/* path as the settings tab being active', () => {
    renderAt('/settings/devices');
    expect(screen.getByRole('button', { name: 'Settings' })).toHaveAttribute(
      'aria-current',
      'page'
    );
  });

  it('navigates when a tab is clicked', async () => {
    renderAt('/human');
    await userEvent.click(screen.getByRole('button', { name: 'Chat' }));
    expect(navigate).toHaveBeenCalledWith('/chat');
    await userEvent.click(screen.getByRole('button', { name: 'Settings' }));
    expect(navigate).toHaveBeenLastCalledWith('/settings');
  });
});
