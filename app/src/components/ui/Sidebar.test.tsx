import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { createRef } from 'react';
import { describe, expect, it, vi } from 'vitest';

import {
  Sidebar,
  SIDEBAR_ICON_WIDTH,
  SIDEBAR_KEYBOARD_STEP,
  type SidebarCollapsible,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarInset,
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuIcon,
  SidebarMenuItem,
  SidebarMenuLabel,
  SidebarProvider,
  SidebarRail,
  SidebarSeparator,
  SidebarTrigger,
  useSidebar,
} from './Sidebar';

const RAW_PALETTE = /\b(bg|text|border|ring)-(neutral|stone|slate|canvas|white|black)\b/;

interface ShellProps {
  collapsible?: SidebarCollapsible;
  defaultOpen?: boolean;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  defaultWidth?: number;
  onWidthChange?: (width: number) => void;
  isActive?: boolean;
}

const renderShell = ({
  collapsible = 'offcanvas',
  isActive = true,
  ...provider
}: ShellProps = {}) =>
  render(
    <SidebarProvider data-testid="provider" {...provider}>
      <Sidebar collapsible={collapsible} data-testid="sidebar">
        <SidebarHeader data-testid="header">
          <SidebarTrigger data-testid="trigger" aria-label="Toggle sidebar" />
        </SidebarHeader>
        <SidebarContent data-testid="content">
          <SidebarGroup data-testid="group">
            <SidebarGroupLabel data-testid="group-label">Main</SidebarGroupLabel>
            <SidebarGroupContent data-testid="group-content">
              <SidebarMenu data-testid="menu">
                <SidebarMenuItem data-testid="item">
                  <SidebarMenuButton data-testid="menu-button" isActive={isActive}>
                    <SidebarMenuIcon data-testid="menu-icon" />
                    <SidebarMenuLabel data-testid="menu-label">Chat</SidebarMenuLabel>
                    <SidebarMenuBadge data-testid="badge" tone="attention">
                      3
                    </SidebarMenuBadge>
                  </SidebarMenuButton>
                </SidebarMenuItem>
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>
        </SidebarContent>
        <SidebarSeparator data-testid="separator" />
        <SidebarFooter data-testid="footer">v1</SidebarFooter>
      </Sidebar>
      <SidebarRail data-testid="rail" aria-label="Resize sidebar" />
      <SidebarInset data-testid="inset">page</SidebarInset>
    </SidebarProvider>
  );

describe('Sidebar', () => {
  it('renders every region with its data-slot contract', () => {
    renderShell();

    const slots: Array<[string, string]> = [
      ['provider', 'sidebar-provider'],
      ['sidebar', 'sidebar'],
      ['header', 'sidebar-header'],
      ['content', 'sidebar-content'],
      ['group', 'sidebar-group'],
      ['group-label', 'sidebar-group-label'],
      ['group-content', 'sidebar-group-content'],
      ['menu', 'sidebar-menu'],
      ['item', 'sidebar-menu-item'],
      ['menu-button', 'sidebar-menu-button'],
      ['menu-icon', 'sidebar-menu-icon'],
      ['menu-label', 'sidebar-menu-label'],
      ['badge', 'sidebar-menu-badge'],
      ['separator', 'sidebar-separator'],
      ['footer', 'sidebar-footer'],
      ['rail', 'sidebar-rail'],
      ['inset', 'sidebar-inset'],
      ['trigger', 'sidebar-trigger'],
    ];
    for (const [testId, slot] of slots) {
      expect(screen.getByTestId(testId)).toHaveAttribute('data-slot', slot);
    }

    expect(screen.getByTestId('sidebar')).toHaveAttribute('data-state', 'expanded');
    expect(screen.getByTestId('sidebar')).toHaveAttribute('data-side', 'left');
    expect(screen.getByTestId('sidebar')).toHaveAttribute('data-collapsible', 'offcanvas');
    expect(screen.getByTestId('menu-button')).toHaveAttribute('data-size', 'md');
    expect(screen.getByTestId('menu-button')).toHaveAttribute('data-active', 'true');
    expect(screen.getByTestId('menu-button')).toHaveAttribute('aria-current', 'page');
    expect(screen.getByTestId('badge')).toHaveAttribute('data-variant', 'attention');
  });

  it('marks an inactive menu button without aria-current', () => {
    renderShell({ isActive: false });

    const button = screen.getByTestId('menu-button');
    expect(button).toHaveAttribute('data-active', 'false');
    expect(button).not.toHaveAttribute('aria-current');
  });

  it('unmounts the column when an offcanvas sidebar collapses', async () => {
    const user = userEvent.setup();
    renderShell();

    expect(screen.getByTestId('sidebar')).toBeInTheDocument();
    await user.click(screen.getByTestId('trigger'));
    expect(screen.queryByTestId('sidebar')).toBeNull();
    // The provider still reports the collapsed state for chrome outside the column.
    expect(screen.getByTestId('provider')).toHaveAttribute('data-state', 'collapsed');
  });

  it('narrows to the icon width instead of unmounting when collapsible="icon"', async () => {
    const user = userEvent.setup();
    renderShell({ collapsible: 'icon', defaultWidth: 240 });

    expect(screen.getByTestId('sidebar')).toHaveStyle({ width: '240px' });
    await user.click(screen.getByTestId('trigger'));
    const sidebar = screen.getByTestId('sidebar');
    expect(sidebar).toHaveAttribute('data-state', 'collapsed');
    expect(sidebar).toHaveStyle({ width: `${SIDEBAR_ICON_WIDTH}px` });
  });

  it('stays expanded when collapsible="none"', async () => {
    const user = userEvent.setup();
    renderShell({ collapsible: 'none', defaultWidth: 240 });

    await user.click(screen.getByTestId('trigger'));
    const sidebar = screen.getByTestId('sidebar');
    expect(sidebar).toHaveAttribute('data-state', 'expanded');
    expect(sidebar).toHaveStyle({ width: '240px' });
  });

  it('reports toggles through onOpenChange and honours a controlled open prop', async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    renderShell({ open: true, onOpenChange });

    await user.click(screen.getByTestId('trigger'));
    expect(onOpenChange).toHaveBeenCalledWith(false);
    // Controlled: the column stays mounted until the caller flips `open`.
    expect(screen.getByTestId('sidebar')).toBeInTheDocument();
  });

  it('toggles from the keyboard on the trigger', async () => {
    const user = userEvent.setup();
    renderShell();

    screen.getByTestId('trigger').focus();
    await user.keyboard('{Enter}');
    expect(screen.queryByTestId('sidebar')).toBeNull();
  });

  it('resizes with the arrow keys on the rail and clamps to the bounds', async () => {
    const user = userEvent.setup();
    const onWidthChange = vi.fn();
    renderShell({ defaultWidth: 240, onWidthChange, collapsible: 'icon' });

    const rail = screen.getByTestId('rail');
    expect(rail).toHaveAttribute('role', 'separator');
    expect(rail).toHaveAttribute('aria-orientation', 'vertical');
    expect(rail).toHaveAttribute('aria-valuenow', '240');

    rail.focus();
    await user.keyboard('{ArrowRight}');
    expect(onWidthChange).toHaveBeenLastCalledWith(240 + SIDEBAR_KEYBOARD_STEP);
    expect(screen.getByTestId('sidebar')).toHaveStyle({
      width: `${240 + SIDEBAR_KEYBOARD_STEP}px`,
    });

    await user.keyboard('{ArrowLeft}{ArrowLeft}');
    expect(screen.getByTestId('sidebar')).toHaveStyle({
      width: `${240 - SIDEBAR_KEYBOARD_STEP}px`,
    });

    // Below the floor the width pins to `minWidth` rather than shrinking away.
    for (let i = 0; i < 20; i += 1) await user.keyboard('{ArrowLeft}');
    expect(onWidthChange).toHaveBeenLastCalledWith(188);
  });

  it('defaults the seam indicator to line-chrome — every current caller paints on the chrome', () => {
    renderShell({ collapsible: 'icon' });
    // The indicator is a nested `<span>`, not the `role="separator"` element
    // itself — assert on the actual child rather than the outer div, which is
    // exactly the seam a `className` override on the rail could never reach.
    const indicator = screen.getByTestId('rail').querySelector('span:last-child');
    const classes = indicator?.className.split(/\s+/) ?? [];
    expect(classes).toContain('group-hover:bg-line-chrome');
    expect(classes).toContain('group-focus:bg-line-chrome');
    // The plain (non-chrome) token, not just any string containing it — the
    // chrome variant legitimately contains "bg-line" as a substring.
    expect(classes).not.toContain('group-hover:bg-line');
    expect(classes).not.toContain('group-focus:bg-line');
  });

  it('lets a caller override the seam indicator via indicatorClassName', () => {
    render(
      <SidebarProvider>
        <SidebarRail
          data-testid="rail"
          aria-label="Resize"
          indicatorClassName="group-hover:bg-line group-focus:bg-line"
        />
      </SidebarProvider>
    );
    const indicator = screen.getByTestId('rail').querySelector('span:last-child');
    expect(indicator?.className).toContain('group-hover:bg-line');
    expect(indicator?.className).not.toContain('bg-line-chrome');
  });

  it('binds mod+B only when keyboardShortcut is enabled', async () => {
    const user = userEvent.setup();
    const { unmount } = render(
      <SidebarProvider data-testid="provider">
        <Sidebar data-testid="sidebar">nav</Sidebar>
      </SidebarProvider>
    );
    await user.keyboard('{Control>}b{/Control}');
    expect(screen.getByTestId('sidebar')).toBeInTheDocument();
    unmount();

    render(
      <SidebarProvider keyboardShortcut data-testid="provider">
        <Sidebar data-testid="sidebar">nav</Sidebar>
      </SidebarProvider>
    );
    await user.keyboard('{Control>}b{/Control}');
    expect(screen.queryByTestId('sidebar')).toBeNull();
  });

  it('exposes state through useSidebar and throws outside a provider', () => {
    const Probe = () => {
      const { state, width, open } = useSidebar();
      return (
        <span data-testid="probe" data-open={String(open)}>
          {state}:{width}
        </span>
      );
    };

    render(
      <SidebarProvider defaultWidth={200}>
        <Probe />
      </SidebarProvider>
    );
    expect(screen.getByTestId('probe')).toHaveTextContent('expanded:200');

    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});
    expect(() => render(<Probe />)).toThrow(/SidebarProvider/);
    consoleError.mockRestore();
  });

  it('renders the caller element under asChild', () => {
    render(
      <SidebarProvider>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton asChild isActive>
              <a href="/chat" data-testid="link">
                Chat
              </a>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarProvider>
    );

    const link = screen.getByTestId('link');
    expect(link.tagName).toBe('A');
    expect(link).toHaveAttribute('data-slot', 'sidebar-menu-button');
    expect(link).toHaveAttribute('aria-current', 'page');
    expect(link).not.toHaveAttribute('type');
  });

  it('forwards ...rest, ids and aria attributes onto the DOM nodes', () => {
    const onClick = vi.fn();
    render(
      <SidebarProvider>
        <Sidebar id="app-sidebar" data-testid="sidebar" aria-label="Primary">
          <SidebarMenu>
            <SidebarMenuItem>
              <SidebarMenuButton
                data-testid="menu-button"
                data-analytics-id="sidebar-chat"
                data-walkthrough="tab-chat"
                title="Chat"
                onClick={onClick}>
                Chat
              </SidebarMenuButton>
            </SidebarMenuItem>
          </SidebarMenu>
        </Sidebar>
      </SidebarProvider>
    );

    expect(screen.getByTestId('sidebar')).toHaveAttribute('id', 'app-sidebar');
    expect(screen.getByTestId('sidebar')).toHaveAttribute('aria-label', 'Primary');

    const button = screen.getByTestId('menu-button');
    expect(button).toHaveAttribute('data-analytics-id', 'sidebar-chat');
    expect(button).toHaveAttribute('data-walkthrough', 'tab-chat');
    expect(button).toHaveAttribute('title', 'Chat');
    expect(button).toHaveAttribute('type', 'button');

    button.click();
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it('forwards refs to the underlying elements', () => {
    const sidebarRef = createRef<HTMLDivElement>();
    const buttonRef = createRef<HTMLButtonElement>();
    const railRef = createRef<HTMLDivElement>();

    render(
      <SidebarProvider>
        <Sidebar ref={sidebarRef}>
          <SidebarMenuButton ref={buttonRef}>Chat</SidebarMenuButton>
        </Sidebar>
        <SidebarRail ref={railRef} aria-label="Resize" />
      </SidebarProvider>
    );

    expect(sidebarRef.current?.tagName).toBe('DIV');
    expect(buttonRef.current?.tagName).toBe('BUTTON');
    expect(railRef.current?.getAttribute('role')).toBe('separator');
  });

  it('lets a caller className win over the default', () => {
    render(
      <SidebarProvider>
        <SidebarInset data-testid="inset" className="rounded-none" />
      </SidebarProvider>
    );

    const inset = screen.getByTestId('inset');
    expect(inset.className).toContain('rounded-none');
    expect(inset.className).not.toMatch(/\brounded-2xl\b/);
  });

  it('drops the card framing when the inset is unframed', () => {
    render(
      <SidebarProvider>
        <SidebarInset data-testid="inset" unframed />
      </SidebarProvider>
    );

    const inset = screen.getByTestId('inset');
    expect(inset).toHaveAttribute('data-unframed', 'true');
    expect(inset.className).not.toMatch(/\brounded-2xl\b/);
  });

  /** Same themeability contract as the rest of the primitive layer. */
  it('never renders a raw palette colour in any collapsible/state combination', () => {
    const modes: SidebarCollapsible[] = ['offcanvas', 'icon', 'none'];
    for (const collapsible of modes) {
      for (const defaultOpen of [true, false]) {
        for (const isActive of [true, false]) {
          const { container, unmount } = renderShell({ collapsible, defaultOpen, isActive });
          for (const el of Array.from(container.querySelectorAll<HTMLElement>('[class]'))) {
            expect(el.className.toString()).not.toMatch(RAW_PALETTE);
          }
          unmount();
        }
      }
    }
  });
});
