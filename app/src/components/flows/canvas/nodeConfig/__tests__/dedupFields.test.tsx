/**
 * Behavior tests for the `dedup` node config form (issue #5263). Covers the
 * single `key` field: it renders and emits a `config.key` patch as the
 * `=`-bindable expression is typed. `useT()` falls back to the bundled
 * English map with no provider mounted (same convention as the sibling
 * `memoryFields.test.tsx`).
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { DedupForm } from '../dedupFields';

function renderDedupForm(config: Record<string, unknown> = {}) {
  const onChange = vi.fn();
  render(<DedupForm config={config} onChange={onChange} />);
  return { onChange };
}

describe('DedupForm', () => {
  it('renders the key field', () => {
    renderDedupForm();
    expect(screen.getByTestId('node-config-dedup-key')).toBeInTheDocument();
  });

  it('seeds the key field from the existing config', () => {
    renderDedupForm({ key: '=item.id' });
    expect(screen.getByTestId('node-config-dedup-key')).toHaveValue('=item.id');
  });

  it('emits a key patch as the expression is typed', () => {
    const { onChange } = renderDedupForm();
    fireEvent.change(screen.getByTestId('node-config-dedup-key'), {
      target: { value: '=item.id' },
    });
    expect(onChange).toHaveBeenLastCalledWith({ key: '=item.id' });
  });
});
