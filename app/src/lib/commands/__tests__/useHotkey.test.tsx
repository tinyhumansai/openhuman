import { act, render } from '@testing-library/react';
import { StrictMode, useState } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { hotkeyManager } from '../hotkeyManager';
import { ScopeContext } from '../ScopeContext';
import { useHotkey } from '../useHotkey';

beforeEach(() => {
  hotkeyManager.teardown();
  hotkeyManager.init();
});

function Wrapper({ children, frame }: { children: React.ReactNode; frame: symbol }) {
  return <ScopeContext.Provider value={frame}>{children}</ScopeContext.Provider>;
}

function TestHotkey({ shortcut, handler }: { shortcut: string; handler: () => void }) {
  useHotkey(shortcut, handler);
  return null;
}

describe('useHotkey', () => {
  it('binds on mount and unbinds on unmount', () => {
    const frame = hotkeyManager.pushFrame('global', 'root');
    const handler = vi.fn();
    const { unmount } = render(
      <Wrapper frame={frame}>
        <TestHotkey shortcut="k" handler={handler} />
      </Wrapper>
    );
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k' }));
    expect(handler).toHaveBeenCalledTimes(1);
    unmount();
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k' }));
    expect(handler).toHaveBeenCalledTimes(1);
    hotkeyManager.popFrame(frame);
  });

  it('StrictMode double-mount yields net 1 binding', () => {
    const frame = hotkeyManager.pushFrame('global', 'root');
    const handler = vi.fn();
    render(
      <StrictMode>
        <Wrapper frame={frame}>
          <TestHotkey shortcut="k" handler={handler} />
        </Wrapper>
      </StrictMode>
    );
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k' }));
    expect(handler).toHaveBeenCalledTimes(1);
    hotkeyManager.popFrame(frame);
  });

  it('handler identity updates via ref without re-registration', () => {
    const frame = hotkeyManager.pushFrame('global', 'root');
    const calls: string[] = [];
    function Inner() {
      const [n, setN] = useState(0);
      useHotkey('k', () => calls.push(`v${n}`));
      return <button onClick={() => setN(v => v + 1)}>bump</button>;
    }
    const { getByText } = render(
      <Wrapper frame={frame}>
        <Inner />
      </Wrapper>
    );
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k' }));
    act(() => {
      getByText('bump').click();
    });
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k' }));
    expect(calls).toEqual(['v0', 'v1']);
    hotkeyManager.popFrame(frame);
  });
});
