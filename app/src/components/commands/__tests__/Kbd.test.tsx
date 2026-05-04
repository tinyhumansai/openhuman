import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import Kbd from '../Kbd';

function withPlatform(value: string, fn: () => void) {
  const orig = navigator.platform;
  Object.defineProperty(navigator, 'platform', { value, configurable: true });
  try {
    fn();
  } finally {
    Object.defineProperty(navigator, 'platform', { value: orig, configurable: true });
  }
}

describe('Kbd', () => {
  it('renders mac glyphs for mod+shift+k', () => {
    withPlatform('MacIntel', () => {
      const { container } = render(<Kbd shortcut="shift+mod+k" />);
      expect(container.textContent).toMatch(/⇧/);
      expect(container.textContent).toMatch(/⌘/);
      expect(container.textContent).toMatch(/K/);
    });
  });

  it('renders PC labels on Win32', () => {
    withPlatform('Win32', () => {
      const { container } = render(<Kbd shortcut="shift+mod+k" />);
      expect(container.textContent).toMatch(/Shift/);
      expect(container.textContent).toMatch(/Ctrl/);
    });
  });

  it('renders single printable', () => {
    withPlatform('MacIntel', () => {
      const { container } = render(<Kbd shortcut="?" />);
      expect(container.textContent).toMatch(/\?/);
    });
  });
});
