import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { createRef } from 'react';
import { describe, expect, it, vi } from 'vitest';

import {
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogRoot,
  AlertDialogTitle,
} from './AlertDialog';

/** Raw palette utilities are banned in this directory by `lint:ui-tokens`. */
const RAW_PALETTE = /\b(bg|text|border|ring)-(neutral|stone|slate|canvas|white|black)\b/;

const renderOpen = (props: Record<string, unknown> = {}) =>
  render(
    <AlertDialogRoot defaultOpen>
      <AlertDialogContent data-testid="alert" {...props}>
        <AlertDialogTitle>Delete workspace</AlertDialogTitle>
        <AlertDialogDescription>This cannot be undone.</AlertDialogDescription>
        <AlertDialogFooter>
          <AlertDialogCancel data-testid="cancel">Cancel</AlertDialogCancel>
          <AlertDialogAction data-testid="action">Delete</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialogRoot>
  );

describe('AlertDialog', () => {
  it('renders an alertdialog with its title, description and both choices', () => {
    renderOpen();

    const content = screen.getByRole('alertdialog');
    expect(content).toHaveAttribute('data-slot', 'alert-dialog-content');
    expect(screen.getByText('Delete workspace')).toHaveAttribute('data-slot', 'alert-dialog-title');
    expect(screen.getByText('This cannot be undone.')).toHaveAttribute(
      'data-slot',
      'alert-dialog-description'
    );
    // Both paths out of the decision must exist — an alert dialog with no
    // cancel leaves Escape as the only escape.
    expect(screen.getByTestId('cancel')).toHaveAttribute('data-slot', 'alert-dialog-cancel');
    expect(screen.getByTestId('action')).toHaveAttribute('data-slot', 'alert-dialog-action');
  });

  it('forwards ...rest and data-testid onto the content node', () => {
    renderOpen({ 'aria-label': 'Confirm deletion', id: 'danger-dialog' });

    const content = screen.getByTestId('alert');
    expect(content).toHaveAttribute('aria-label', 'Confirm deletion');
    expect(content).toHaveAttribute('id', 'danger-dialog');
  });

  it('forwards a ref to the content element', () => {
    const ref = createRef<HTMLDivElement>();
    render(
      <AlertDialogRoot defaultOpen>
        <AlertDialogContent ref={ref}>
          <AlertDialogTitle>Title</AlertDialogTitle>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction>Go</AlertDialogAction>
        </AlertDialogContent>
      </AlertDialogRoot>
    );
    expect(ref.current).toBe(screen.getByRole('alertdialog'));
  });

  it('defaults the action to the danger tone and honours an explicit one', () => {
    render(
      <AlertDialogRoot defaultOpen>
        <AlertDialogContent>
          <AlertDialogTitle>Title</AlertDialogTitle>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction data-testid="a">Delete</AlertDialogAction>
        </AlertDialogContent>
      </AlertDialogRoot>
    );
    expect(screen.getByTestId('a')).toHaveAttribute('data-tone', 'danger');
  });

  it('marks a benign confirm with the default tone', () => {
    render(
      <AlertDialogRoot defaultOpen>
        <AlertDialogContent>
          <AlertDialogTitle>Title</AlertDialogTitle>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction tone="default" data-testid="a">
            Continue
          </AlertDialogAction>
        </AlertDialogContent>
      </AlertDialogRoot>
    );
    expect(screen.getByTestId('a')).toHaveAttribute('data-tone', 'default');
  });

  it('closes on Escape and reports it through onOpenChange', () => {
    const onOpenChange = vi.fn();
    render(
      <AlertDialogRoot defaultOpen onOpenChange={onOpenChange}>
        <AlertDialogContent>
          <AlertDialogTitle>Title</AlertDialogTitle>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction>Delete</AlertDialogAction>
        </AlertDialogContent>
      </AlertDialogRoot>
    );

    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it('activates Cancel from the keyboard — Radix focuses it on open', async () => {
    const user = userEvent.setup();
    const onCancel = vi.fn();
    render(
      <AlertDialogRoot defaultOpen>
        <AlertDialogContent>
          <AlertDialogTitle>Title</AlertDialogTitle>
          <AlertDialogCancel data-testid="cancel" onClick={onCancel}>
            Cancel
          </AlertDialogCancel>
          <AlertDialogAction data-testid="action">Delete</AlertDialogAction>
        </AlertDialogContent>
      </AlertDialogRoot>
    );

    // The escape hatch is the default focus target, deliberately: an alert
    // dialog must not put a destructive button under a stray Enter.
    expect(screen.getByTestId('cancel')).toHaveFocus();
    await user.keyboard('{Enter}');
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it('reaches the Action with Tab and activates it from the keyboard', async () => {
    const user = userEvent.setup();
    const onAction = vi.fn();
    render(
      <AlertDialogRoot defaultOpen>
        <AlertDialogContent>
          <AlertDialogTitle>Title</AlertDialogTitle>
          <AlertDialogCancel data-testid="cancel">Cancel</AlertDialogCancel>
          <AlertDialogAction data-testid="action" onClick={onAction}>
            Delete
          </AlertDialogAction>
        </AlertDialogContent>
      </AlertDialogRoot>
    );

    await user.tab();
    expect(screen.getByTestId('action')).toHaveFocus();
    await user.keyboard('{Enter}');
    expect(onAction).toHaveBeenCalledTimes(1);
  });

  it('does NOT dismiss on an outside click — the decision must be answered', () => {
    const onOpenChange = vi.fn();
    render(
      <AlertDialogRoot defaultOpen onOpenChange={onOpenChange}>
        <AlertDialogContent>
          <AlertDialogTitle>Title</AlertDialogTitle>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction>Delete</AlertDialogAction>
        </AlertDialogContent>
      </AlertDialogRoot>
    );

    fireEvent.pointerDown(document.body);
    fireEvent.mouseDown(document.body);
    fireEvent.click(document.body);

    expect(onOpenChange).not.toHaveBeenCalled();
    expect(screen.getByRole('alertdialog')).toBeInTheDocument();
  });

  it('portals into the container it is given, not document.body', () => {
    const container = document.createElement('div');
    document.body.appendChild(container);

    renderOpen({ container });

    // Transparent mascot/notch/overlay windows render their own root; a scrim
    // in document.body would paint over a see-through window.
    expect(container.contains(screen.getByRole('alertdialog'))).toBe(true);
  });

  it('lets a caller className win over the defaults', () => {
    renderOpen({ className: 'max-w-lg' });
    expect(screen.getByTestId('alert').className).toContain('max-w-lg');
    expect(screen.getByTestId('alert').className).not.toContain('max-w-sm');
  });

  it('emits no raw palette utility on any slot', () => {
    renderOpen();

    const nodes = [
      screen.getByTestId('alert'),
      screen.getByTestId('cancel'),
      screen.getByTestId('action'),
      screen.getByText('Delete workspace'),
      screen.getByText('This cannot be undone.'),
      document.querySelector('[data-slot="alert-dialog-overlay"]'),
      document.querySelector('[data-slot="alert-dialog-footer"]'),
    ];

    for (const node of nodes) {
      expect(node).not.toBeNull();
      expect((node as Element).className).not.toMatch(RAW_PALETTE);
    }
  });
});
