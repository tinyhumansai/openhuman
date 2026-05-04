import { expect } from 'vitest';

export interface PressKeyOptions {
  key: string;
  mod?: boolean;
  shift?: boolean;
  alt?: boolean;
  ctrl?: boolean;
  target?: EventTarget;
}

export function pressKey(opts: PressKeyOptions): KeyboardEvent {
  const mac = navigator.platform.toLowerCase().includes('mac');
  const modPair = opts.mod ? (mac ? { metaKey: true } : { ctrlKey: true }) : {};
  const target = opts.target ?? window;
  const event = new KeyboardEvent('keydown', {
    key: opts.key,
    bubbles: true,
    cancelable: true,
    shiftKey: !!opts.shift,
    altKey: !!opts.alt,
    ctrlKey: !!opts.ctrl,
    ...modPair,
  });
  target.dispatchEvent(event);
  return event;
}

export function __metaAssertPressKeyReachesCaptureListener(): void {
  let reached = false;
  const listener = (_e: KeyboardEvent) => {
    reached = true;
  };
  window.addEventListener('keydown', listener, { capture: true });
  pressKey({ key: 'z' });
  window.removeEventListener('keydown', listener, { capture: true });
  expect(reached).toBe(true);
}
