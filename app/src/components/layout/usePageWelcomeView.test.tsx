import { fireEvent, render, screen } from '@testing-library/react';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { describe, expect, it } from 'vitest';

import { SidebarSlotOutlet, SidebarSlotProvider } from './shell/SidebarSlot';
import { usePageWelcomeView } from './usePageWelcomeView';

function WorkflowViews({ landOnWelcome = false }: { landOnWelcome?: boolean }) {
  const location = useLocation();
  const { view, nav } = usePageWelcomeView({
    landOnWelcome,
    ariaLabel: 'Workflows',
    welcomeLabel: 'Introduction',
    mainLabel: 'My workflows',
    mainIconPath: 'M1 1h3',
    defaultView: 'main',
  });

  return (
    <>
      <output data-testid="active-workflow-view">{view}</output>
      <output data-testid="workflow-view-url">{location.search}</output>
      {nav}
      <SidebarSlotOutlet />
    </>
  );
}

function renderAt(entry: string, landOnWelcome = false) {
  render(
    <MemoryRouter initialEntries={[entry]}>
      <SidebarSlotProvider>
        <WorkflowViews landOnWelcome={landOnWelcome} />
      </SidebarSlotProvider>
    </MemoryRouter>
  );
}

describe('usePageWelcomeView when the working view is default', () => {
  it('opens the working view without a URL parameter', () => {
    renderAt('/flows');
    expect(screen.getByTestId('active-workflow-view')).toHaveTextContent('main');
  });

  it('can still navigate to and directly link to the introduction', () => {
    renderAt('/flows');
    fireEvent.click(screen.getByTestId('two-pane-nav-welcome'));
    expect(screen.getByTestId('active-workflow-view')).toHaveTextContent('welcome');
    expect(screen.getByTestId('workflow-view-url')).toHaveTextContent('?view=welcome');
  });

  it('respects an explicit introductory URL even with a main default', () => {
    renderAt('/flows?view=welcome');
    expect(screen.getByTestId('active-workflow-view')).toHaveTextContent('welcome');
    expect(screen.getByTestId('two-pane-nav-welcome')).toHaveAttribute('aria-current', 'page');
  });
});

describe('usePageWelcomeView when landing on welcome (first visit)', () => {
  it('lands on welcome without a URL parameter', () => {
    renderAt('/flows', true);
    expect(screen.getByTestId('active-workflow-view')).toHaveTextContent('welcome');
    expect(screen.getByTestId('two-pane-nav-welcome')).toHaveAttribute('aria-current', 'page');
  });

  it('moves to the working view and does not snap back to welcome', () => {
    renderAt('/flows', true);
    fireEvent.click(screen.getByTestId('two-pane-nav-main'));
    expect(screen.getByTestId('active-workflow-view')).toHaveTextContent('main');
    expect(screen.getByTestId('workflow-view-url')).toBeEmptyDOMElement();
  });

  it('lets an explicit ?view= win over the welcome landing', () => {
    renderAt('/flows?view=main', true);
    expect(screen.getByTestId('active-workflow-view')).toHaveTextContent('main');
  });
});
