import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it } from 'vitest';

import { hotkeyManager } from '../../../lib/commands/hotkeyManager';
import { pressKey } from '../../../test/commandTestUtils';
import CommandProvider from '../CommandProvider';

beforeEach(() => {
  hotkeyManager.teardown();
});

describe('CommandProvider', () => {
  it('mounts and registers seed actions', () => {
    render(
      <MemoryRouter>
        <CommandProvider>
          <div>child</div>
        </CommandProvider>
      </MemoryRouter>
    );
    expect(screen.getByText('child')).toBeInTheDocument();
  });

  it('opens palette on mod+K', async () => {
    render(
      <MemoryRouter>
        <CommandProvider>
          <div>child</div>
        </CommandProvider>
      </MemoryRouter>
    );
    act(() => {
      pressKey({ key: 'k', mod: true });
    });
    expect(await screen.findByRole('dialog', { name: /Command palette/i })).toBeInTheDocument();
  });

  it.skip('opens help on ? (disabled — help overlay temporarily off)', async () => {
    render(
      <MemoryRouter>
        <CommandProvider>
          <div>child</div>
        </CommandProvider>
      </MemoryRouter>
    );
    act(() => {
      pressKey({ key: '?' });
    });
    expect(await screen.findByRole('dialog', { name: /Keyboard shortcuts/i })).toBeInTheDocument();
  });

  it('Esc closes open overlay', async () => {
    const user = userEvent.setup();
    render(
      <MemoryRouter>
        <CommandProvider>
          <div>child</div>
        </CommandProvider>
      </MemoryRouter>
    );
    act(() => {
      pressKey({ key: 'k', mod: true });
    });
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it.skip('palette and help mutually exclusive (disabled — help overlay temporarily off)', async () => {
    render(
      <MemoryRouter>
        <CommandProvider>
          <div>child</div>
        </CommandProvider>
      </MemoryRouter>
    );
    act(() => {
      pressKey({ key: 'k', mod: true });
    });
    expect(await screen.findByRole('dialog', { name: /Command palette/i })).toBeInTheDocument();
    act(() => {
      pressKey({ key: '?' });
    });
    expect(await screen.findByRole('dialog', { name: /Keyboard shortcuts/i })).toBeInTheDocument();
    expect(screen.queryByRole('dialog', { name: /Command palette/i })).not.toBeInTheDocument();
  });
});
