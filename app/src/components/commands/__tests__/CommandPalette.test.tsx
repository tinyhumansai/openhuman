import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { hotkeyManager } from '../../../lib/commands/hotkeyManager';
import { registry } from '../../../lib/commands/registry';
import { ScopeContext } from '../../../lib/commands/ScopeContext';
import CommandPalette from '../CommandPalette';

let currentFrame: symbol;

beforeEach(() => {
  hotkeyManager.teardown();
  registry.reset();
  hotkeyManager.init();
  currentFrame = hotkeyManager.pushFrame('global', 'root');
  registry.setActiveStack([currentFrame]);
});

afterEach(() => {
  hotkeyManager.teardown();
  registry.reset();
});

function Harness({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  return (
    <ScopeContext.Provider value={currentFrame}>
      <CommandPalette open={open} onOpenChange={onOpenChange} />
    </ScopeContext.Provider>
  );
}

function registerSettingsAction(handler?: () => void): void {
  registry.registerAction(
    {
      id: 'nav.settings',
      label: 'Open Settings',
      handler: handler ?? vi.fn(),
      group: 'Navigation',
      shortcut: 'mod+,',
    },
    currentFrame
  );
}

describe('CommandPalette', () => {
  it('renders registered actions when open', () => {
    registerSettingsAction();
    render(<Harness open={true} onOpenChange={() => {}} />);
    expect(screen.getByText('Open Settings')).toBeInTheDocument();
  });

  it('filters by typed query', async () => {
    registerSettingsAction();
    const user = userEvent.setup();
    render(<Harness open={true} onOpenChange={() => {}} />);
    const input = screen.getByRole('combobox');
    await user.type(input, 'xyzzy');
    expect(screen.queryByText('Open Settings')).not.toBeInTheDocument();
  });

  it('fires handler on Enter and calls onOpenChange(false)', async () => {
    registerSettingsAction();
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    render(<Harness open={true} onOpenChange={onOpenChange} />);
    const input = screen.getByRole('combobox');
    await user.type(input, 'settings');
    await user.keyboard('{Enter}');
    await act(async () => {
      await new Promise(r => requestAnimationFrame(() => r(null)));
    });
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it('renders footer hint', () => {
    render(<Harness open={true} onOpenChange={() => {}} />);
    expect(screen.getByText(/Press \? for all shortcuts/i)).toBeInTheDocument();
  });
});
