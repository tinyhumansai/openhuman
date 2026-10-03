import { cleanup, render, screen } from '@testing-library/react';
import { MemoryRouter, Route, Routes, useLocation } from 'react-router-dom';
import { describe, expect, it } from 'vitest';

import BrainRedirect from './BrainRedirect';

function Where() {
  const { pathname, search, hash } = useLocation();
  return <div data-testid="where">{`${pathname}${search}${hash}`}</div>;
}

function renderAt(entry: string) {
  // Several cases render more than once per test; start each from a clean DOM.
  cleanup();
  render(
    <MemoryRouter initialEntries={[entry]}>
      <Routes>
        <Route path="/brain" element={<BrainRedirect />} />
        <Route path="/connections" element={<Where />} />
      </Routes>
    </MemoryRouter>
  );
  return screen.getByTestId('where').textContent;
}

describe('BrainRedirect', () => {
  it('lands bare /brain on the Memory tab of Connections', () => {
    expect(renderAt('/brain')).toBe('/connections?tab=brain');
  });

  it('maps the v1 graph and goals sub-tabs to the ask chip', () => {
    expect(renderAt('/brain?tab=graph')).toBe('/connections?tab=brain&brain=ask');
    expect(renderAt('/brain?tab=goals')).toBe('/connections?tab=brain&brain=ask');
  });

  it('maps the v1 sources and sync sub-tabs to the documents chip', () => {
    expect(renderAt('/brain?tab=sources')).toBe('/connections?tab=brain&brain=documents');
    expect(renderAt('/brain?tab=sync')).toBe('/connections?tab=brain&brain=documents');
  });

  it('passes a v2 chip through unchanged', () => {
    expect(renderAt('/brain?tab=context')).toBe('/connections?tab=brain&brain=context');
  });

  it('drops a sub-tab that names no chip so the page picks its default', () => {
    expect(renderAt('/brain?tab=welcome')).toBe('/connections?tab=brain');
  });

  it('maps the sync history view to documents and keeps other params and the hash', () => {
    expect(renderAt('/brain?tab=sync&view=history&foo=1#x')).toBe(
      '/connections?tab=brain&brain=documents&foo=1#x'
    );
  });
});
