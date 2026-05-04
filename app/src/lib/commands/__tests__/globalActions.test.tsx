import type { NavigateFunction } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { GROUP_ORDER, registerGlobalActions } from '../globalActions';
import { hotkeyManager } from '../hotkeyManager';
import { registry } from '../registry';

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
  it('registers the 5 seed nav actions into the global frame', () => {
    const frame = hotkeyManager.pushFrame('global', 'root');
    const navigate = vi.fn() as unknown as NavigateFunction;
    registerGlobalActions(navigate, frame);
    const ids = ['nav.home', 'nav.chat', 'nav.intelligence', 'nav.skills', 'nav.settings'];
    for (const id of ids) expect(registry.getAction(id)?.id).toBe(id);
    expect(registry.getAction('help.show')).toBeUndefined();
  });

  it('nav.home handler calls navigate("/home")', () => {
    const frame = hotkeyManager.pushFrame('global', 'root');
    const navigate = vi.fn();
    registerGlobalActions(navigate as unknown as NavigateFunction, frame);
    registry.setActiveStack([frame]);
    registry.runAction('nav.home');
    expect(navigate).toHaveBeenCalledWith('/home');
  });

  it('exports GROUP_ORDER', () => {
    expect(GROUP_ORDER).toEqual(['Navigation']);
  });
});
