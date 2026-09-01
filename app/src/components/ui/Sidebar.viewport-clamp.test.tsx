import { act, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import { Sidebar, SidebarProvider, SidebarRail } from './Sidebar';

/**
 * The viewport clamp (#5907) and the single-source-of-truth property that
 * makes it correct (#5941).
 *
 * The sidebar width was clamped against SIDEBAR_MIN_WIDTH/SIDEBAR_MAX_WIDTH and
 * never against the window, so a narrow window left the column owning most of
 * it. The first fix was a CSS `max-w-[50vw]`, which review correctly rejected:
 * CSS constrains the *rendered* box while every consumer of the stored value —
 * the rail's drag origin, its arrow-key step, its `aria-valuenow` — keeps
 * reading the larger number. With a persisted 420 in a 414px viewport the
 * column renders 207px, dragging fully left proposes ~213 so `max-width` pins
 * it at 207, and the sidebar cannot be narrowed at all.
 *
 * The clamp now lives in `SidebarProvider`, so context hands one width to
 * everything. These tests assert exactly that agreement — the rendered box AND
 * the separator's reported value, together, because the defect was the two
 * disagreeing.
 */

const ORIGINAL_INNER_WIDTH = window.innerWidth;

function setViewport(width: number) {
  Object.defineProperty(window, 'innerWidth', { configurable: true, writable: true, value: width });
  act(() => {
    window.dispatchEvent(new Event('resize'));
  });
}

afterEach(() => {
  Object.defineProperty(window, 'innerWidth', {
    configurable: true,
    writable: true,
    value: ORIGINAL_INNER_WIDTH,
  });
});

const renderSidebar = (width: number) =>
  render(
    <SidebarProvider defaultWidth={width}>
      <Sidebar data-testid="sidebar" collapsible="none" />
      <SidebarRail data-testid="rail" aria-label="Resize sidebar" />
    </SidebarProvider>
  );

describe('Sidebar viewport clamp', () => {
  it('is inert at a desktop width — the stored width decides', () => {
    setViewport(1280);
    renderSidebar(420);

    // 50% of 1280 is 640, well above the 420 cap, so the clamp cannot bind.
    expect(screen.getByTestId('sidebar')).toHaveStyle({ width: '420px' });
    expect(screen.getByTestId('rail')).toHaveAttribute('aria-valuenow', '420');
  });

  it('caps the column at half a narrow window', () => {
    setViewport(414);
    renderSidebar(420);

    // floor(414 / 2) = 207.
    expect(screen.getByTestId('sidebar')).toHaveStyle({ width: '207px' });
  });

  it('reports the clamped width to assistive tech, not the stored one', () => {
    // The half of #5941 that a CSS-only clamp could never have fixed: the
    // separator announced 420 for a 207px column.
    setViewport(414);
    renderSidebar(420);

    expect(screen.getByTestId('rail')).toHaveAttribute('aria-valuenow', '207');
  });

  it('announces the reachable ceiling, not the configured one', () => {
    // The same class of defect as the `aria-valuenow` one, one level up: the
    // context published the raw 420 as the maximum, so the separator advertised
    // a ceiling the column cannot reach at this viewport (#5941, CodeRabbit).
    setViewport(414);
    renderSidebar(420);

    expect(screen.getByTestId('rail')).toHaveAttribute('aria-valuemax', '207');
  });

  it('never goes below the minimum, however narrow the window', () => {
    // Below 2 * minWidth a fraction stops being meaningful; the floor wins so
    // the two clamps cannot disagree.
    setViewport(200);
    renderSidebar(420);

    expect(screen.getByTestId('sidebar')).toHaveStyle({ width: '188px' });
  });

  it('re-clamps when the window is resized, and restores on widening', () => {
    // The listener is the reason this is not a CSS rule. It also shows the
    // stored preference surviving: 420 comes back when there is room for it.
    setViewport(1280);
    renderSidebar(420);
    expect(screen.getByTestId('sidebar')).toHaveStyle({ width: '420px' });

    setViewport(414);
    expect(screen.getByTestId('sidebar')).toHaveStyle({ width: '207px' });

    setViewport(1280);
    expect(screen.getByTestId('sidebar')).toHaveStyle({ width: '420px' });
  });
});
