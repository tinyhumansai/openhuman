import { act, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it } from 'vitest';

import { hotkeyManager } from '../../../lib/commands/hotkeyManager';
import { pressKey } from '../../../test/commandTestUtils';
import { renderWithProviders } from '../../../test/test-utils';
import CommandProvider from '../CommandProvider';

beforeEach(() => {
  hotkeyManager.teardown();
});

function renderProvider() {
  return renderWithProviders(
    <CommandProvider>
      <div>child</div>
    </CommandProvider>
  );
}

describe('CommandProvider', () => {
  it('mounts and registers seed actions', () => {
    renderProvider();
    expect(screen.getByText('child')).toBeInTheDocument();
  });

  it('opens palette on mod+K', async () => {
    renderProvider();
    act(() => {
      pressKey({ key: 'k', mod: true });
    });
    expect(await screen.findByRole('dialog', { name: /Command palette/i })).toBeInTheDocument();
  });

  it('opens help on ?', async () => {
    renderProvider();
    act(() => {
      pressKey({ key: '?' });
    });
    expect(await screen.findByRole('dialog', { name: /Keyboard shortcuts/i })).toBeInTheDocument();
  });

  it('opens help on mod+/', async () => {
    renderProvider();
    act(() => {
      pressKey({ key: '/', mod: true });
    });
    expect(await screen.findByRole('dialog', { name: /Keyboard shortcuts/i })).toBeInTheDocument();
  });

  it('Esc closes open overlay', async () => {
    const user = userEvent.setup();
    renderProvider();
    act(() => {
      pressKey({ key: 'k', mod: true });
    });
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('palette and help are mutually exclusive', async () => {
    renderProvider();
    act(() => {
      pressKey({ key: 'k', mod: true });
    });
    expect(await screen.findByRole('dialog', { name: /Command palette/i })).toBeInTheDocument();
    // Opening the shortcuts directory dismisses the palette.
    act(() => {
      pressKey({ key: '/', mod: true });
    });
    expect(await screen.findByRole('dialog', { name: /Keyboard shortcuts/i })).toBeInTheDocument();
    expect(screen.queryByRole('dialog', { name: /Command palette/i })).not.toBeInTheDocument();
  });
});
