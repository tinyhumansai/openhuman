/**
 * OrchestrationRedirect maps the retired `/orchestration` route (and its legacy
 * `?tab=`/`?sub=`/`?session=` query) onto Brain's `/brain?tab=orchestration`
 * with the `?ov=`/`?sub=` scheme. Rendered under a MemoryRouter with a `/brain`
 * sink route that reports the resolved URL so each mapping branch is asserted.
 */
import { render, screen } from '@testing-library/react';
import { MemoryRouter, Route, Routes, useLocation } from 'react-router-dom';
import { describe, expect, it } from 'vitest';

import { OrchestrationRedirect } from '../../AppRoutes';

function BrainProbe() {
  const { pathname, search } = useLocation();
  return <div data-testid="brain">{`${pathname}${search}`}</div>;
}

const resolve = (from: string): string => {
  render(
    <MemoryRouter initialEntries={[from]}>
      <Routes>
        <Route path="/orchestration" element={<OrchestrationRedirect />} />
        <Route path="/brain" element={<BrainProbe />} />
      </Routes>
    </MemoryRouter>
  );
  const target = screen.getByTestId('brain').textContent ?? '';
  const params = new URLSearchParams(target.split('?')[1] ?? '');
  return `${target.split('?')[0]}?${params.toString()}`;
};

describe('OrchestrationRedirect', () => {
  it('bare /orchestration → the orchestration tab (no view override)', () => {
    const params = new URLSearchParams(resolve('/orchestration').split('?')[1]);
    expect(params.get('tab')).toBe('orchestration');
    expect(params.get('ov')).toBeNull();
  });

  it.each(['agent', 'overview', 'tasks', 'network', 'medulla'])('legacy ?tab=%s → ?ov=%s', view => {
    const params = new URLSearchParams(resolve(`/orchestration?tab=${view}`).split('?')[1]);
    expect(params.get('tab')).toBe('orchestration');
    expect(params.get('ov')).toBe(view);
  });

  it.each(['connections', 'discover', 'usage'])('legacy ?tab=%s → ?ov=network&sub=%s', sub => {
    const params = new URLSearchParams(resolve(`/orchestration?tab=${sub}`).split('?')[1]);
    expect(params.get('ov')).toBe('network');
    expect(params.get('sub')).toBe(sub);
  });

  it('preserves a network ?sub= when landing on ?tab=network', () => {
    const params = new URLSearchParams(
      resolve('/orchestration?tab=network&sub=usage').split('?')[1]
    );
    expect(params.get('ov')).toBe('network');
    expect(params.get('sub')).toBe('usage');
  });

  it('preserves ?session= for the agent chat', () => {
    const params = new URLSearchParams(
      resolve('/orchestration?tab=agent&session=sess-1').split('?')[1]
    );
    expect(params.get('ov')).toBe('agent');
    expect(params.get('session')).toBe('sess-1');
  });

  it('ignores an unknown ?tab= (falls back to the orchestration tab)', () => {
    const params = new URLSearchParams(resolve('/orchestration?tab=bogus').split('?')[1]);
    expect(params.get('tab')).toBe('orchestration');
    expect(params.get('ov')).toBeNull();
  });
});
