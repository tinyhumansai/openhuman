/**
 * Behavior tests for the `memory` node config form (issue #5226). Covers the
 * per-operation progressive disclosure (`recall`/`search` vs `flavour` vs
 * `remember`/`forget`) and the hard UI rule mirroring the engine invariant:
 * a `remember`/`forget` node's `scope` control must never offer `user`.
 * `useT()` falls back to the bundled English map with no provider mounted
 * (same convention as the sibling `nodeConfigForms.test.tsx`).
 */
import { fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { MemoryForm } from '../memoryFields';

function renderMemoryForm(config: Record<string, unknown> = {}) {
  const onChange = vi.fn();
  render(<MemoryForm config={config} onChange={onChange} />);
  return { onChange };
}

describe('MemoryForm', () => {
  it('defaults to recall: shows scope (including user) and query, no key/value/flavour', () => {
    renderMemoryForm();
    const scope = screen.getByTestId('node-config-memory-scope');
    const options = within(scope).getAllByRole('option');
    expect(options.map(o => o.getAttribute('value'))).toEqual(['user', 'flow', 'flows']);
    expect(screen.getByTestId('node-config-memory-query')).toBeInTheDocument();
    expect(screen.queryByTestId('node-config-memory-flavour')).not.toBeInTheDocument();
    expect(screen.queryByTestId('node-config-memory-key')).not.toBeInTheDocument();
    expect(screen.queryByTestId('node-config-memory-value')).not.toBeInTheDocument();
  });

  it('search behaves like recall: scope includes user, and query is shown', () => {
    renderMemoryForm({ operation: 'search' });
    const scope = screen.getByTestId('node-config-memory-scope');
    const options = within(scope).getAllByRole('option');
    expect(options.map(o => o.getAttribute('value'))).toContain('user');
    expect(screen.getByTestId('node-config-memory-query')).toBeInTheDocument();
  });

  it('flavour shows only the flavour slug field, no scope or query', () => {
    renderMemoryForm({ operation: 'flavour' });
    expect(screen.getByTestId('node-config-memory-flavour')).toBeInTheDocument();
    expect(screen.queryByTestId('node-config-memory-scope')).not.toBeInTheDocument();
    expect(screen.queryByTestId('node-config-memory-query')).not.toBeInTheDocument();
  });

  it('remember hides the user scope option (only flow) and shows key + value', () => {
    renderMemoryForm({ operation: 'remember', scope: 'flow' });
    const scope = screen.getByTestId('node-config-memory-scope');
    const options = within(scope).getAllByRole('option');
    expect(options.map(o => o.getAttribute('value'))).toEqual(['flow']);
    expect(within(scope).queryByRole('option', { name: /read-only/i })).not.toBeInTheDocument();
    expect(screen.getByTestId('node-config-memory-key')).toBeInTheDocument();
    expect(screen.getByTestId('node-config-memory-value')).toBeInTheDocument();
    // Not a read/search operation, so no query field.
    expect(screen.queryByTestId('node-config-memory-query')).not.toBeInTheDocument();
  });

  it('forget hides the user scope option and shows key but not value', () => {
    renderMemoryForm({ operation: 'forget', scope: 'flow' });
    const scope = screen.getByTestId('node-config-memory-scope');
    const options = within(scope).getAllByRole('option');
    expect(options.map(o => o.getAttribute('value'))).toEqual(['flow']);
    expect(screen.getByTestId('node-config-memory-key')).toBeInTheDocument();
    expect(screen.queryByTestId('node-config-memory-value')).not.toBeInTheDocument();
  });

  it('clamps scope to flow when switching from recall(user) to remember', () => {
    const { onChange } = renderMemoryForm({ operation: 'recall', scope: 'user' });
    fireEvent.change(screen.getByTestId('node-config-memory-operation'), {
      target: { value: 'remember' },
    });
    expect(onChange).toHaveBeenLastCalledWith({ operation: 'remember', scope: 'flow' });
  });

  it('self-heals a pre-existing invalid remember+user config on mount', () => {
    const { onChange } = renderMemoryForm({ operation: 'remember', scope: 'user' });
    expect(onChange).toHaveBeenCalledWith({ scope: 'flow' });
  });

  it('emits a query patch as the query is typed for recall', () => {
    const { onChange } = renderMemoryForm({ operation: 'recall' });
    fireEvent.change(screen.getByTestId('node-config-memory-query'), {
      target: { value: '=item.title' },
    });
    expect(onChange).toHaveBeenLastCalledWith({ query: '=item.title' });
  });

  it('emits limit as a number as it is typed', () => {
    const { onChange } = renderMemoryForm({ operation: 'recall' });
    fireEvent.change(screen.getByTestId('node-config-memory-limit'), { target: { value: '5' } });
    expect(onChange).toHaveBeenLastCalledWith({ limit: 5 });
  });

  it('emits undefined for limit when the field is cleared back to empty', () => {
    // Start from an already-populated `limit` so clearing it is a genuine
    // DOM value change (a fresh controlled input already renders blank).
    const { onChange } = renderMemoryForm({ operation: 'recall', limit: 5 });
    fireEvent.change(screen.getByTestId('node-config-memory-limit'), { target: { value: '' } });
    expect(onChange).toHaveBeenLastCalledWith({ limit: undefined });
  });

  it('emits min_score as a number as it is typed', () => {
    const { onChange } = renderMemoryForm({ operation: 'recall' });
    fireEvent.change(screen.getByTestId('node-config-memory-min-score'), {
      target: { value: '0.5' },
    });
    expect(onChange).toHaveBeenLastCalledWith({ min_score: 0.5 });
  });

  it('emits undefined for min_score when the field is cleared back to empty', () => {
    // Start from an already-populated `min_score` so clearing it is a genuine
    // DOM value change (a fresh controlled input already renders blank).
    const { onChange } = renderMemoryForm({ operation: 'recall', min_score: 0.5 });
    fireEvent.change(screen.getByTestId('node-config-memory-min-score'), { target: { value: '' } });
    expect(onChange).toHaveBeenLastCalledWith({ min_score: undefined });
  });
});
