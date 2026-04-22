import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import HelpOverlay from '../HelpOverlay';
import { ScopeContext } from '../../../lib/commands/ScopeContext';
import { hotkeyManager } from '../../../lib/commands/hotkeyManager';
import { registry } from '../../../lib/commands/registry';

beforeEach(() => {
  hotkeyManager.teardown();
  hotkeyManager.init();
});

describe('HelpOverlay', () => {
  it('shows actions section with registered action shortcuts', () => {
    const f = hotkeyManager.pushFrame('global', 'root');
    registry.setActiveStack([f]);
    registry.registerAction(
      {
        id: 'nav.home',
        label: 'Go Home',
        group: 'Navigation',
        handler: vi.fn(),
        shortcut: 'mod+1',
      },
      f,
    );
    hotkeyManager.bind(f, { shortcut: 'mod+1', handler: vi.fn(), id: 'nav.home' });
    render(
      <ScopeContext.Provider value={f}>
        <HelpOverlay open={true} onOpenChange={() => {}} />
      </ScopeContext.Provider>,
    );
    expect(screen.getByText('Go Home')).toBeInTheDocument();
    expect(screen.getByText(/Actions/i)).toBeInTheDocument();
  });

  it('shows bare HotkeyBinding with description in Shortcuts section', () => {
    const f = hotkeyManager.pushFrame('global', 'root');
    hotkeyManager.bind(f, { shortcut: 'mod+/', handler: vi.fn(), description: 'Toggle foo' });
    render(
      <ScopeContext.Provider value={f}>
        <HelpOverlay open={true} onOpenChange={() => {}} />
      </ScopeContext.Provider>,
    );
    expect(screen.getByText('Toggle foo')).toBeInTheDocument();
    expect(screen.getByText(/Shortcuts/i)).toBeInTheDocument();
  });

  it('dedups same shortcut across scopes', () => {
    const g = hotkeyManager.pushFrame('global', 'root');
    const p = hotkeyManager.pushFrame('page', 'home');
    registry.registerAction(
      { id: 'nav.home', label: 'Go Home global', handler: vi.fn(), shortcut: 'mod+1' },
      g,
    );
    registry.registerAction(
      { id: 'nav.home', label: 'Go Home page', handler: vi.fn(), shortcut: 'mod+1' },
      p,
    );
    hotkeyManager.bind(g, { shortcut: 'mod+1', handler: vi.fn(), id: 'nav.home' });
    hotkeyManager.bind(p, { shortcut: 'mod+1', handler: vi.fn(), id: 'nav.home' });
    render(
      <ScopeContext.Provider value={p}>
        <HelpOverlay open={true} onOpenChange={() => {}} />
      </ScopeContext.Provider>,
    );
    const matches = screen.queryAllByText(/Go Home/);
    expect(matches.length).toBe(1);
    expect(matches[0].textContent).toContain('page');
  });
});
