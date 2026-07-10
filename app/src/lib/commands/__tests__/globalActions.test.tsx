import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { type GlobalActionHandlers, GROUP_ORDER, registerGlobalActions } from '../globalActions';
import { hotkeyManager } from '../hotkeyManager';
import { registry } from '../registry';

function makeHandlers(overrides: Partial<GlobalActionHandlers> = {}): GlobalActionHandlers {
  return {
    navigate: vi.fn(),
    newChat: vi.fn(),
    toggleSidebar: vi.fn(),
    openPalette: vi.fn(),
    openShortcuts: vi.fn(),
    ...overrides,
  };
}

beforeEach(() => {
  hotkeyManager.teardown();
  registry.reset();
  hotkeyManager.init();
});

afterEach(() => {
  hotkeyManager.teardown();
  registry.reset();
});

describe('registerGlobalActions', () => {
  it('registers the nav seed actions plus the new chat/view/general actions', () => {
    const frame = hotkeyManager.pushFrame('global', 'root');
    registerGlobalActions(makeHandlers(), frame);
    const ids = [
      'nav.home',
      'nav.chat',
      'nav.intelligence',
      'nav.skills',
      'nav.activity',
      'nav.settings',
      'chat.new',
      'view.toggle-sidebar',
      'meta.command-palette',
      'meta.keyboard-shortcuts',
    ];
    for (const id of ids) expect(registry.getAction(id)?.id).toBe(id);
    expect(registry.getAction('help.show')).toBeUndefined();
  });

  it('nav.home handler calls navigate("/home")', () => {
    const frame = hotkeyManager.pushFrame('global', 'root');
    const navigate = vi.fn();
    registerGlobalActions(makeHandlers({ navigate }), frame);
    registry.setActiveStack([frame]);
    registry.runAction('nav.home');
    expect(navigate).toHaveBeenCalledWith('/home');
  });

  it('routes chat/view/general actions to their handlers', () => {
    const frame = hotkeyManager.pushFrame('global', 'root');
    const newChat = vi.fn();
    const toggleSidebar = vi.fn();
    const openPalette = vi.fn();
    const openShortcuts = vi.fn();
    registerGlobalActions(
      makeHandlers({ newChat, toggleSidebar, openPalette, openShortcuts }),
      frame
    );
    registry.setActiveStack([frame]);

    registry.runAction('chat.new');
    expect(newChat).toHaveBeenCalledTimes(1);
    registry.runAction('view.toggle-sidebar');
    expect(toggleSidebar).toHaveBeenCalledTimes(1);
    registry.runAction('meta.command-palette');
    expect(openPalette).toHaveBeenCalledTimes(1);
    registry.runAction('meta.keyboard-shortcuts');
    expect(openShortcuts).toHaveBeenCalledTimes(1);
  });

  it('binds alias keys (mod+p → palette, ? → shortcuts) to the same handlers', () => {
    const frame = hotkeyManager.pushFrame('global', 'root');
    const openPalette = vi.fn();
    const openShortcuts = vi.fn();
    registerGlobalActions(makeHandlers({ openPalette, openShortcuts }), frame);

    // mod+p alias (use ctrl on the non-mac CI default).
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'p', ctrlKey: true, bubbles: true }));
    expect(openPalette).toHaveBeenCalledTimes(1);

    // '?' alias opens the shortcuts directory.
    window.dispatchEvent(new KeyboardEvent('keydown', { key: '?', bubbles: true }));
    expect(openShortcuts).toHaveBeenCalledTimes(1);
  });

  it('disposer unwinds every registered action', () => {
    const frame = hotkeyManager.pushFrame('global', 'root');
    const dispose = registerGlobalActions(makeHandlers(), frame);
    expect(registry.getAction('chat.new')?.id).toBe('chat.new');
    dispose();
    expect(registry.getAction('chat.new')).toBeUndefined();
    expect(registry.getAction('nav.home')).toBeUndefined();
  });

  it('exports GROUP_ORDER', () => {
    expect(GROUP_ORDER).toEqual(['Navigation', 'Profiles', 'Chat', 'View', 'General']);
  });

  it('wires profile-switch shortcuts but keeps them hidden (disabled) until profiles exist', () => {
    const frame = hotkeyManager.pushFrame('global', 'root');
    registerGlobalActions(makeHandlers(), frame);
    registry.setActiveStack([frame]);
    // Hidden: not surfaced in the active (registry) action list…
    expect(registry.getAction('profile.switch-1')).toBeUndefined();
    // …and inert: the binding exists but is disabled, so running it is a no-op
    // that never throws.
    expect(() => registry.runAction('profile.switch-1')).not.toThrow();
  });
});
