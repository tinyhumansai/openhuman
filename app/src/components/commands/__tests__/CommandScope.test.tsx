import { describe, it, expect, beforeEach } from 'vitest';
import { render } from '@testing-library/react';
import { StrictMode } from 'react';
import CommandScope from '../CommandScope';
import { hotkeyManager } from '../../../lib/commands/hotkeyManager';

beforeEach(() => {
  hotkeyManager.teardown();
  hotkeyManager.init();
});

describe('CommandScope', () => {
  it('pushes frame on mount, pops on unmount', () => {
    const { unmount } = render(
      <CommandScope id="home">
        <div />
      </CommandScope>,
    );
    expect(hotkeyManager.getStackSymbols().length).toBe(1);
    unmount();
    expect(hotkeyManager.getStackSymbols().length).toBe(0);
  });

  it('StrictMode double-mount nets a single frame', () => {
    render(
      <StrictMode>
        <CommandScope id="home">
          <div />
        </CommandScope>
      </StrictMode>,
    );
    expect(hotkeyManager.getStackSymbols().length).toBe(1);
  });

  it('nested scopes push two frames', () => {
    render(
      <CommandScope id="page">
        <CommandScope id="modal" kind="modal">
          <div />
        </CommandScope>
      </CommandScope>,
    );
    expect(hotkeyManager.getStackSymbols().length).toBe(2);
  });

  it('pops by symbol (out-of-order unmount safe)', () => {
    function App({ showInner }: { showInner: boolean }) {
      return (
        <CommandScope id="outer">
          {showInner && (
            <CommandScope id="inner">
              <div />
            </CommandScope>
          )}
        </CommandScope>
      );
    }
    const { rerender, unmount } = render(<App showInner={true} />);
    expect(hotkeyManager.getStackSymbols().length).toBe(2);
    rerender(<App showInner={false} />);
    expect(hotkeyManager.getStackSymbols().length).toBe(1);
    unmount();
    expect(hotkeyManager.getStackSymbols().length).toBe(0);
  });
});
