import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, test, vi } from 'vitest';

import { ModalShell } from './ModalShell';

/**
 * The dialog is backed by Radix, which portals the overlay and the panel as
 * siblings — so the backdrop is no longer the panel's `parentElement`. These
 * helpers name the contract ("the thing a user clicks to dismiss") instead of a
 * particular DOM shape, so the tests survive the next implementation swap too.
 */
/**
 * Radix registers its outside-pointer listener in a macrotask, so the click
 * that opened a dialog cannot immediately close it again. Tests that act
 * synchronously after render observe no listener at all, so they flush first.
 */
async function flushDeferredWork(): Promise<void> {
  await new Promise(resolve => setTimeout(resolve, 0));
}

/**
 * Radix treats an outside interaction as pointerdown *followed by* a click —
 * it waits for the click so that dragging out of a dialog does not dismiss it.
 * Firing pointerdown alone dismisses nothing, which is why this helper exists
 * rather than a bare `fireEvent.pointerDown`.
 */
function dismissByOutsideClick(): void {
  fireEvent.pointerDown(overlay());
  fireEvent.click(overlay());
}

function overlay(): HTMLElement {
  const element = document.querySelector('[data-slot="dialog-overlay"]');
  if (!element) throw new Error('dialog overlay not rendered');
  return element as HTMLElement;
}

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

function renderModal(props: Partial<React.ComponentProps<typeof ModalShell>> = {}) {
  const onClose = vi.fn();
  const result = render(
    <ModalShell title="Dialog title" titleId="dialog-title" onClose={onClose} {...props}>
      <p>Dialog content</p>
    </ModalShell>
  );
  return { ...result, onClose };
}

describe('ModalShell', () => {
  test('moves focus into the dialog and restores prior focus on unmount', async () => {
    const trigger = document.createElement('button');
    document.body.appendChild(trigger);
    trigger.focus();

    const { unmount } = renderModal();
    // Radix moves focus to the first focusable element inside the panel rather
    // than to the panel itself, which is why this asserts containment rather
    // than `toHaveFocus()` on the dialog node.
    expect(screen.getByRole('dialog').contains(document.activeElement)).toBe(true);

    unmount();
    await waitFor(() => expect(trigger).toHaveFocus());
    trigger.remove();
  });

  test('closes from Escape and an outside pointer when permitted', async () => {
    const { onClose } = renderModal();

    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);

    await flushDeferredWork();
    dismissByOutsideClick();
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  test('independently applies explicit close policy fields', async () => {
    const { onClose } = renderModal({
      closePolicy: { escape: false, backdrop: false, button: true },
    });

    await flushDeferredWork();
    fireEvent.keyDown(document, { key: 'Escape' });
    dismissByOutsideClick();
    expect(onClose).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: 'common.close' }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  test('disables Escape, backdrop, and button closing and omits the close button', async () => {
    const { onClose } = renderModal({
      closePolicy: { escape: false, backdrop: false, button: false },
    });

    await flushDeferredWork();
    fireEvent.keyDown(document, { key: 'Escape' });
    dismissByOutsideClick();

    expect(onClose).not.toHaveBeenCalled();
    expect(screen.queryByRole('button', { name: 'common.close' })).not.toBeInTheDocument();
  });

  test('does not dismiss from pointer input inside the dialog panel', () => {
    const { onClose } = renderModal();

    fireEvent.pointerDown(screen.getByText('Dialog content'));

    expect(onClose).not.toHaveBeenCalled();
  });

  test('renders the footer in a dedicated slot after the content', () => {
    renderModal({ footer: <button>Footer action</button> });

    const content = screen.getByText('Dialog content').parentElement!;
    const footer = screen.getByRole('button', { name: 'Footer action' }).parentElement!;
    expect(footer).toHaveClass('border-t', 'border-line-subtle', 'px-5', 'py-4');
    expect(content.nextElementSibling).toBe(footer);
  });

  test('forwards an explicit aria-describedby value', () => {
    renderModal({ describedBy: 'dialog-description' });

    expect(screen.getByRole('dialog')).toHaveAttribute('aria-describedby', 'dialog-description');
  });

  test('forwards testId onto the panel that carries role="dialog"', () => {
    renderModal({ testId: 'add-account-modal' });

    expect(screen.getByTestId('add-account-modal')).toBe(screen.getByRole('dialog'));
  });

  test('emits no data-testid attribute when testId is omitted', () => {
    renderModal();

    expect(screen.getByRole('dialog')).not.toHaveAttribute('data-testid');
  });
});
