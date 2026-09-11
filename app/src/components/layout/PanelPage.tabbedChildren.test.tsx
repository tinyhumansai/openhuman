/**
 * Regression: a tabbed `PanelPage` silently dropped `children`.
 *
 * The tabbed branch rendered only the active tab, so anything passed as
 * children vanished with no type error and no warning. Tabbing the AI settings
 * page moved a save bar and four dialogs into that `children`, and the only
 * symptom was that picking a provider appeared to do nothing at all.
 *
 * A `children?: never` union was the first attempt and was abandoned: it does
 * not hold, because TypeScript does not enforce it against JSX children (tried,
 * then verified by mutation rather than assumed). Rendering them is what
 * actually removes the failure mode, so that is what this pins.
 */
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import PanelPage from './PanelPage';

describe('PanelPage children', () => {
  it('renders children alongside the active tab body', () => {
    render(
      <PanelPage
        value="one"
        tabs={[
          { id: 'one', label: 'One', content: <p>tab one body</p> },
          { id: 'two', label: 'Two', content: <p>tab two body</p> },
        ]}>
        <div data-testid="page-overlay">a dialog belonging to every tab</div>
      </PanelPage>
    );

    expect(screen.getByText('tab one body')).toBeInTheDocument();
    expect(screen.getByTestId('page-overlay')).toBeInTheDocument();
  });

  it('keeps children mounted when the active tab changes', () => {
    const { rerender } = render(
      <PanelPage
        value="one"
        tabs={[
          { id: 'one', label: 'One', content: <p>tab one body</p> },
          { id: 'two', label: 'Two', content: <p>tab two body</p> },
        ]}>
        <div data-testid="page-overlay">overlay</div>
      </PanelPage>
    );
    expect(screen.getByTestId('page-overlay')).toBeInTheDocument();

    rerender(
      <PanelPage
        value="two"
        tabs={[
          { id: 'one', label: 'One', content: <p>tab one body</p> },
          { id: 'two', label: 'Two', content: <p>tab two body</p> },
        ]}>
        <div data-testid="page-overlay">overlay</div>
      </PanelPage>
    );

    expect(screen.getByText('tab two body')).toBeInTheDocument();
    expect(screen.getByTestId('page-overlay')).toBeInTheDocument();
  });

  it('still renders children as the body when there are no tabs', () => {
    render(
      <PanelPage>
        <p>single body</p>
      </PanelPage>
    );
    expect(screen.getByText('single body')).toBeInTheDocument();
  });
});
