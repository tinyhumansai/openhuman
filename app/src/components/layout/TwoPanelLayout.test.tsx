import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import TwoPanelLayout, { useTwoPanelLayout } from './TwoPanelLayout';

function Sidebar() {
  return <div>sidebar-content</div>;
}
function Content() {
  return <div>main-content</div>;
}

describe('TwoPanelLayout', () => {
  it('renders only the content when the sidebar is hidden', () => {
    renderWithProviders(
      <TwoPanelLayout id="chat" sidebar={<Sidebar />}>
        <Content />
      </TwoPanelLayout>,
      {
        preloadedState: {
          layout: { panels: { chat: { sidebarVisible: false, sidebarWidth: 256 } } },
        },
      }
    );
    expect(screen.getByText('main-content')).toBeInTheDocument();
    expect(screen.queryByText('sidebar-content')).not.toBeInTheDocument();
    expect(screen.queryByTestId('two-panel-divider-chat')).not.toBeInTheDocument();
  });

  it('renders sidebar + divider when open and applies the persisted width', () => {
    renderWithProviders(
      <TwoPanelLayout id="chat" sidebar={<Sidebar />}>
        <Content />
      </TwoPanelLayout>,
      {
        preloadedState: {
          layout: { panels: { chat: { sidebarVisible: true, sidebarWidth: 300 } } },
        },
      }
    );
    expect(screen.getByText('sidebar-content')).toBeInTheDocument();
    const pane = screen.getByTestId('two-panel-sidebar-chat');
    expect(pane).toHaveStyle({ width: '300px' });
    expect(screen.getByTestId('two-panel-divider-chat')).toBeInTheDocument();
  });

  it('forceSidebarVisible overrides a hidden persisted state without mutating it', () => {
    const { store } = renderWithProviders(
      <TwoPanelLayout id="chat" sidebar={<Sidebar />} forceSidebarVisible>
        <Content />
      </TwoPanelLayout>,
      {
        preloadedState: {
          layout: { panels: { chat: { sidebarVisible: false, sidebarWidth: 256 } } },
        },
      }
    );
    expect(screen.getByText('sidebar-content')).toBeInTheDocument();
    expect(
      (store.getState() as { layout: { panels: Record<string, unknown> } }).layout.panels.chat
    ).toEqual({ sidebarVisible: false, sidebarWidth: 256 });
  });

  it('resizes via keyboard and clamps to the configured bounds', () => {
    const { store } = renderWithProviders(
      <TwoPanelLayout id="chat" sidebar={<Sidebar />} minSidebarWidth={180} maxSidebarWidth={480}>
        <Content />
      </TwoPanelLayout>,
      {
        preloadedState: {
          layout: { panels: { chat: { sidebarVisible: true, sidebarWidth: 300 } } },
        },
      }
    );
    const divider = screen.getByTestId('two-panel-divider-chat');
    const widthOf = () =>
      (store.getState() as { layout: { panels: Record<string, { sidebarWidth: number }> } }).layout
        .panels.chat.sidebarWidth;

    fireEvent.keyDown(divider, { key: 'ArrowRight' });
    expect(widthOf()).toBe(316);
    fireEvent.keyDown(divider, { key: 'ArrowLeft' });
    expect(widthOf()).toBe(300);
  });

  it('seeds default geometry for an unseen panel via ensurePanelLayout', () => {
    const { store } = renderWithProviders(
      <TwoPanelLayout
        id="fresh"
        sidebar={<Sidebar />}
        defaultSidebarVisible
        defaultSidebarWidth={222}>
        <Content />
      </TwoPanelLayout>
    );
    const panel = (store.getState() as { layout: { panels: Record<string, unknown> } }).layout
      .panels.fresh;
    expect(panel).toEqual({ sidebarVisible: true, sidebarWidth: 222 });
  });

  it('shows a reopen rail when collapsed and showCollapsedRail is set', () => {
    const { store } = renderWithProviders(
      <TwoPanelLayout id="chat" sidebar={<Sidebar />} showCollapsedRail>
        <Content />
      </TwoPanelLayout>,
      {
        preloadedState: {
          layout: { panels: { chat: { sidebarVisible: false, sidebarWidth: 256 } } },
        },
      }
    );
    const reopen = screen.getByTestId('two-panel-reopen-chat');
    fireEvent.click(reopen);
    expect(
      (store.getState() as { layout: { panels: Record<string, { sidebarVisible: boolean }> } })
        .layout.panels.chat.sidebarVisible
    ).toBe(true);
  });
});

describe('useTwoPanelLayout', () => {
  function Harness() {
    const { sidebarVisible, toggleSidebar } = useTwoPanelLayout('chat', { sidebarVisible: false });
    return (
      <button type="button" onClick={toggleSidebar}>
        {sidebarVisible ? 'open' : 'closed'}
      </button>
    );
  }

  it('reflects and toggles the same persisted state', () => {
    renderWithProviders(<Harness />);
    const btn = screen.getByRole('button');
    expect(btn).toHaveTextContent('closed');
    fireEvent.click(btn);
    expect(btn).toHaveTextContent('open');
    fireEvent.click(btn);
    expect(btn).toHaveTextContent('closed');
  });
});
