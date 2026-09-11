import { render, screen } from '@testing-library/react';
import { describe, expect, test } from 'vitest';

import { SheetContent, SheetRoot, SheetTitle } from './Sheet';

/**
 * `SheetContent` is the destination for the app's hand-rolled drawers, each of
 * which hangs a `data-testid` on both its panel and its scrim
 * (`flow-runs-drawer` / `flow-runs-backdrop`,
 * `flow-run-inspector-drawer` / `-backdrop`). These pin that both hooks have
 * somewhere to land, and that a sheet rendering today is unchanged.
 */
function renderSheet(props: Partial<React.ComponentProps<typeof SheetContent>> = {}) {
  return render(
    <SheetRoot open>
      <SheetContent aria-describedby={undefined} {...props}>
        <SheetTitle>Drawer title</SheetTitle>
        <p>Drawer body</p>
      </SheetContent>
    </SheetRoot>
  );
}

function overlay(): HTMLElement {
  const element = document.querySelector('[data-slot="dialog-overlay"]');
  if (!element) throw new Error('sheet overlay not rendered');
  return element as HTMLElement;
}

describe('SheetContent', () => {
  test('forwards testId onto the panel and overlayTestId onto the scrim', () => {
    renderSheet({ testId: 'flow-runs-drawer', overlayTestId: 'flow-runs-backdrop' });

    expect(screen.getByTestId('flow-runs-drawer')).toBe(screen.getByRole('dialog'));
    expect(screen.getByTestId('flow-runs-backdrop')).toBe(overlay());
  });

  test('emits neither attribute when the props are omitted', () => {
    renderSheet();

    expect(screen.getByRole('dialog')).not.toHaveAttribute('data-testid');
    expect(overlay()).not.toHaveAttribute('data-testid');
  });

  /**
   * The three drawers already on this primitive pass `data-testid` straight
   * through `...rest`. The spread runs after `testId`, so their value must
   * still be the one that lands — anything else is a silent breakage of
   * `subagent-drawer`, `background-processes-panel` and
   * `agent-process-source-panel`.
   */
  test('an explicit data-testid still wins over testId', () => {
    render(
      <SheetRoot open>
        <SheetContent aria-describedby={undefined} testId="ignored" data-testid="subagent-drawer">
          <SheetTitle>Drawer title</SheetTitle>
        </SheetContent>
      </SheetRoot>
    );

    expect(screen.getByRole('dialog')).toHaveAttribute('data-testid', 'subagent-drawer');
    expect(screen.queryByTestId('ignored')).not.toBeInTheDocument();
  });

  test('leaves the side variant and slot markers untouched', () => {
    renderSheet({ side: 'left', testId: 'side-sheet' });

    const panel = screen.getByTestId('side-sheet');
    expect(panel).toHaveAttribute('data-slot', 'sheet-content');
    expect(panel).toHaveAttribute('data-side', 'left');
  });
});
