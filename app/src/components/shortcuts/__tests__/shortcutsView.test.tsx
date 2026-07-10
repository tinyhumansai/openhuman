import { render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { hotkeyManager } from '../../../lib/commands/hotkeyManager';
import { registry } from '../../../lib/commands/registry';
import { ShortcutsList } from '../shortcutsView';

beforeEach(() => {
  hotkeyManager.teardown();
  registry.reset();
  hotkeyManager.init();
});

afterEach(() => {
  hotkeyManager.teardown();
  registry.reset();
});

function seed(): symbol {
  const frame = hotkeyManager.pushFrame('global', 'root');
  // Registered out of display order on purpose — the view must re-order them.
  registry.registerAction(
    { id: 'd', label: 'Command Palette', group: 'General', shortcut: 'mod+k', handler() {} },
    frame
  );
  registry.registerAction(
    { id: 'c', label: 'Toggle Sidebar', group: 'View', shortcut: 'mod+b', handler() {} },
    frame
  );
  registry.registerAction(
    { id: 'a', label: 'Go to Chat', group: 'Navigation', shortcut: 'mod+2', handler() {} },
    frame
  );
  registry.registerAction(
    { id: 'b', label: 'New Chat', group: 'Chat', shortcut: 'mod+n', handler() {} },
    frame
  );
  registry.setActiveStack([frame]);
  return frame;
}

describe('ShortcutsList', () => {
  it('groups shortcuts and orders the groups Navigation → Chat → View → General', () => {
    seed();
    render(<ShortcutsList />);
    const headings = screen.getAllByRole('heading').map(h => h.textContent);
    expect(headings).toEqual(['Navigation', 'Chat', 'View', 'General']);
    expect(screen.getByText('New Chat')).toBeInTheDocument();
    expect(screen.getByText('Toggle Sidebar')).toBeInTheDocument();
  });

  it('omits actions that have no shortcut', () => {
    const frame = hotkeyManager.pushFrame('global', 'root');
    registry.registerAction(
      { id: 'with', label: 'Has Shortcut', group: 'Navigation', shortcut: 'mod+2', handler() {} },
      frame
    );
    registry.registerAction(
      { id: 'without', label: 'No Shortcut', group: 'Navigation', handler() {} },
      frame
    );
    registry.setActiveStack([frame]);

    render(<ShortcutsList />);
    expect(screen.getByText('Has Shortcut')).toBeInTheDocument();
    expect(screen.queryByText('No Shortcut')).not.toBeInTheDocument();
  });

  it('renders an empty state when no shortcuts are active', () => {
    render(<ShortcutsList />);
    expect(screen.getByText(/no keyboard shortcuts/i)).toBeInTheDocument();
  });
});
