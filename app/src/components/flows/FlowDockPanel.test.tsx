/**
 * FlowDockPanel (Workflows UI redesign, Piece 1) — asserts the tabbed dock's
 * core contract: both tab bodies stay mounted at all times (only `display`
 * toggles), tab clicks flip `activeTab` via the host callback, the Run tab is
 * disabled when `runTabDisabled`, the collapse button calls `onCollapse`, and
 * dragging the resize handle grows/shrinks the dock's width.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import FlowDockPanel, { type FlowDockTab } from './FlowDockPanel';

// A tiny stateful probe standing in for `WorkflowCopilotPanel` — a real mount
// counter proves the component is never unmounted/remounted across a tab
// switch (the #4942 / #5008 / #5010 regression this dock exists to avoid).
let copilotMountCount = 0;
function CopilotProbe() {
  useState(() => {
    copilotMountCount += 1;
    return null;
  });
  return <div data-testid="copilot-probe">copilot content</div>;
}

function renderDock(overrides: Partial<React.ComponentProps<typeof FlowDockPanel>> = {}) {
  const onTabChange = overrides.onTabChange ?? vi.fn();
  const onCollapse = overrides.onCollapse ?? vi.fn();
  const utils = render(
    <FlowDockPanel
      activeTab="copilot"
      onTabChange={onTabChange}
      copilotContent={<CopilotProbe />}
      runContent={<div data-testid="run-probe">run content</div>}
      onCollapse={onCollapse}
      {...overrides}
    />
  );
  return { ...utils, onTabChange, onCollapse };
}

/** A thin stateful wrapper so `activeTab` actually flips in response to `onTabChange`, mirroring how `FlowEditor` drives it. */
function ControlledDock({ initialTab = 'copilot' as FlowDockTab } = {}) {
  const [tab, setTab] = useState<FlowDockTab>(initialTab);
  return (
    <FlowDockPanel
      activeTab={tab}
      onTabChange={setTab}
      copilotContent={<CopilotProbe />}
      runContent={<div data-testid="run-probe">run content</div>}
      onCollapse={vi.fn()}
    />
  );
}

describe('FlowDockPanel', () => {
  beforeEach(() => {
    copilotMountCount = 0;
  });

  it('renders both tab bodies in the DOM at once (only display toggles) — Copilot stays mounted across tab switches', () => {
    render(<ControlledDock />);

    expect(screen.getByTestId('copilot-probe')).toBeInTheDocument();
    expect(screen.getByTestId('run-probe')).toBeInTheDocument();
    expect(copilotMountCount).toBe(1);

    fireEvent.click(screen.getByTestId('flow-dock-tab-run'));
    // Both bodies are STILL in the DOM — the Run tab's body becomes visible,
    // the Copilot's becomes `display: none`, but neither unmounts.
    expect(screen.getByTestId('copilot-probe')).toBeInTheDocument();
    expect(screen.getByTestId('run-probe')).toBeInTheDocument();
    expect(copilotMountCount).toBe(1); // still exactly one mount — no remount

    fireEvent.click(screen.getByTestId('flow-dock-tab-copilot'));
    expect(copilotMountCount).toBe(1);
  });

  it('hides the inactive tab body via display:none, not by unmounting', () => {
    render(<ControlledDock />);

    expect(screen.getByTestId('flow-dock-copilot-body')).toHaveStyle({ display: 'flex' });
    expect(screen.getByTestId('flow-dock-run-body')).toHaveStyle({ display: 'none' });

    fireEvent.click(screen.getByTestId('flow-dock-tab-run'));

    expect(screen.getByTestId('flow-dock-copilot-body')).toHaveStyle({ display: 'none' });
    expect(screen.getByTestId('flow-dock-run-body')).toHaveStyle({ display: 'block' });
  });

  it('calls onTabChange with the clicked tab', () => {
    const { onTabChange } = renderDock();
    fireEvent.click(screen.getByTestId('flow-dock-tab-run'));
    expect(onTabChange).toHaveBeenCalledWith('run');
  });

  it('marks the active tab aria-selected', () => {
    renderDock({ activeTab: 'run' });
    expect(screen.getByTestId('flow-dock-tab-run')).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByTestId('flow-dock-tab-copilot')).toHaveAttribute('aria-selected', 'false');
  });

  it('disables the Run tab when runTabDisabled (a draft has no runs)', () => {
    renderDock({ runTabDisabled: true });
    expect(screen.getByTestId('flow-dock-tab-run')).toBeDisabled();
  });

  it('calls onCollapse when the collapse button is clicked', () => {
    const { onCollapse } = renderDock();
    fireEvent.click(screen.getByTestId('flow-dock-collapse'));
    expect(onCollapse).toHaveBeenCalledTimes(1);
  });

  it('omits the resize handle in fullWidth mode', () => {
    renderDock({ fullWidth: true });
    expect(screen.queryByTestId('flow-dock-resize-handle')).not.toBeInTheDocument();
    expect(screen.getByTestId('flow-dock-panel')).toHaveAttribute('data-full-width', 'true');
  });

  it('grows the panel width when the resize handle is dragged toward the canvas (left)', () => {
    renderDock();
    const panel = screen.getByTestId('flow-dock-panel');
    const initialWidth = parseInt(panel.style.width, 10);

    fireEvent.pointerDown(screen.getByTestId('flow-dock-resize-handle'), { clientX: 500 });
    fireEvent.pointerMove(window, { clientX: 400 }); // dragged 100px left -> +100px wider
    fireEvent.pointerUp(window);

    const widthAfter = parseInt(panel.style.width, 10);
    expect(widthAfter).toBe(initialWidth + 100);
  });

  it('clamps the resized width within [320, 560]', () => {
    renderDock();
    const panel = screen.getByTestId('flow-dock-panel');

    fireEvent.pointerDown(screen.getByTestId('flow-dock-resize-handle'), { clientX: 500 });
    fireEvent.pointerMove(window, { clientX: -5000 }); // absurdly far left
    fireEvent.pointerUp(window);
    expect(parseInt(panel.style.width, 10)).toBe(560);

    fireEvent.pointerDown(screen.getByTestId('flow-dock-resize-handle'), { clientX: 500 });
    fireEvent.pointerMove(window, { clientX: 5000 }); // absurdly far right
    fireEvent.pointerUp(window);
    expect(parseInt(panel.style.width, 10)).toBe(320);
  });
});
