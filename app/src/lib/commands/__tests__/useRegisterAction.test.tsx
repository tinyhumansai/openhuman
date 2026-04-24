import { render } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { hotkeyManager } from '../hotkeyManager';
import { registry } from '../registry';
import { ScopeContext } from '../ScopeContext';
import { useRegisterAction } from '../useRegisterAction';

beforeEach(() => {
  hotkeyManager.teardown();
  hotkeyManager.init();
});

function Wrapper({ children, frame }: { children: React.ReactNode; frame: symbol }) {
  return <ScopeContext.Provider value={frame}>{children}</ScopeContext.Provider>;
}

function TestAction({
  id,
  shortcut,
  handler,
}: {
  id: string;
  shortcut?: string;
  handler: () => void;
}) {
  useRegisterAction({ id, label: id, handler, shortcut });
  return null;
}

describe('useRegisterAction', () => {
  it('adds to registry on mount, removes on unmount', () => {
    const frame = hotkeyManager.pushFrame('global', 'root');
    registry.setActiveStack([frame]);
    const handler = vi.fn();
    const { unmount } = render(
      <Wrapper frame={frame}>
        <TestAction id="x.y" handler={handler} />
      </Wrapper>
    );
    expect(registry.getAction('x.y')?.id).toBe('x.y');
    unmount();
    expect(registry.getAction('x.y')).toBeUndefined();
    hotkeyManager.popFrame(frame);
    registry.setActiveStack([]);
  });

  it('with shortcut: fires via keydown AND registers in registry', () => {
    const frame = hotkeyManager.pushFrame('global', 'root');
    registry.setActiveStack([frame]);
    const handler = vi.fn();
    render(
      <Wrapper frame={frame}>
        <TestAction id="x.y" shortcut="k" handler={handler} />
      </Wrapper>
    );
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k' }));
    expect(handler).toHaveBeenCalled();
    expect(registry.getAction('x.y')).toBeDefined();
    hotkeyManager.popFrame(frame);
    registry.setActiveStack([]);
  });
});
