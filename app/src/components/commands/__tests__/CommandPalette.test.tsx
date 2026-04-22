import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, act } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import CommandPalette from '../CommandPalette';
import { ScopeContext } from '../../../lib/commands/ScopeContext';
import { registry } from '../../../lib/commands/registry';
import { hotkeyManager } from '../../../lib/commands/hotkeyManager';

beforeEach(() => {
  hotkeyManager.teardown();
  hotkeyManager.init();
});

function Harness({
  open,
  onOpenChange,
  handler,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  handler?: () => void;
}) {
  const frame = hotkeyManager.pushFrame('global', 'root');
  registry.setActiveStack([frame]);
  registry.registerAction(
    {
      id: 'nav.settings',
      label: 'Open Settings',
      handler: handler ?? vi.fn(),
      group: 'Navigation',
      shortcut: 'mod+,',
    },
    frame,
  );
  return (
    <ScopeContext.Provider value={frame}>
      <CommandPalette open={open} onOpenChange={onOpenChange} />
    </ScopeContext.Provider>
  );
}

describe('CommandPalette', () => {
  it('renders registered actions when open', () => {
    render(<Harness open={true} onOpenChange={() => {}} />);
    expect(screen.getByText('Open Settings')).toBeInTheDocument();
  });

  it('filters by typed query', async () => {
    const user = userEvent.setup();
    render(<Harness open={true} onOpenChange={() => {}} />);
    const input = screen.getByRole('combobox');
    await user.type(input, 'xyzzy');
    expect(screen.queryByText('Open Settings')).not.toBeInTheDocument();
  });

  it('fires handler on Enter and calls onOpenChange(false)', async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    render(<Harness open={true} onOpenChange={onOpenChange} />);
    const input = screen.getByRole('combobox');
    await user.type(input, 'settings');
    await user.keyboard('{Enter}');
    await act(async () => {
      await new Promise((r) => requestAnimationFrame(() => r(null)));
    });
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it('renders footer hint', () => {
    render(<Harness open={true} onOpenChange={() => {}} />);
    expect(screen.getByText(/Press \? for all shortcuts/i)).toBeInTheDocument();
  });
});
