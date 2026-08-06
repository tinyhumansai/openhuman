import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, test, vi } from 'vitest';

import { ModalShell } from './ModalShell';

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
  test('moves focus into the dialog and restores prior focus on unmount', () => {
    const trigger = document.createElement('button');
    document.body.appendChild(trigger);
    trigger.focus();

    const { unmount } = renderModal();
    expect(screen.getByRole('dialog')).toHaveFocus();

    unmount();
    expect(trigger).toHaveFocus();
    trigger.remove();
  });

  test('closes from Escape and an outside pointer when permitted', () => {
    const { onClose } = renderModal();

    fireEvent.keyDown(document, { key: 'Escape' });
    fireEvent.pointerDown(screen.getByRole('dialog').parentElement!);

    expect(onClose).toHaveBeenCalledTimes(2);
  });

  test('independently applies explicit close policy fields', () => {
    const { onClose } = renderModal({
      closePolicy: { escape: false, backdrop: false, button: true },
    });

    fireEvent.keyDown(document, { key: 'Escape' });
    fireEvent.pointerDown(screen.getByRole('dialog').parentElement!);
    expect(onClose).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: 'common.close' }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  test('disables Escape, backdrop, and button closing and omits the close button', () => {
    const { onClose } = renderModal({
      closePolicy: { escape: false, backdrop: false, button: false },
    });

    fireEvent.keyDown(document, { key: 'Escape' });
    fireEvent.pointerDown(screen.getByRole('dialog').parentElement!);

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
});
