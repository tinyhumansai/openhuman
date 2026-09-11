import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, test, vi } from 'vitest';

import { ConfirmDialog } from './ConfirmDialog';

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

function baseProps() {
  return {
    title: 'Confirm action',
    body: 'Continue with this action?',
    onConfirm: vi.fn(),
    onCancel: vi.fn(),
  };
}

describe('ConfirmDialog', () => {
  test('renders string and React-node bodies', () => {
    const { rerender } = render(<ConfirmDialog {...baseProps()} />);
    expect(screen.getByText('Continue with this action?')).toBeInTheDocument();

    rerender(
      <ConfirmDialog
        {...baseProps()}
        body={
          <p>
            Rich <strong>body</strong>
          </p>
        }
      />
    );
    expect(screen.getByText('Rich', { exact: false })).toContainElement(screen.getByText('body'));
  });

  test('uses neutral confirmation tone by default and danger tone when destructive', () => {
    const { rerender } = render(<ConfirmDialog {...baseProps()} />);
    expect(screen.getByRole('button', { name: 'common.confirm' })).toHaveClass('bg-primary-500');

    rerender(<ConfirmDialog {...baseProps()} destructive />);
    expect(screen.getByRole('button', { name: 'common.confirm' })).toHaveClass('bg-coral-500');
  });

  test('calls confirmation and cancellation actions', async () => {
    const props = baseProps();
    const user = userEvent.setup();
    render(<ConfirmDialog {...props} />);

    expect(screen.getByTestId('confirm-dialog-confirm')).toBe(
      screen.getByRole('button', { name: 'common.confirm' })
    );
    await user.click(screen.getByRole('button', { name: 'common.confirm' }));
    await user.click(screen.getByRole('button', { name: 'common.cancel' }));

    expect(props.onConfirm).toHaveBeenCalledTimes(1);
    expect(props.onCancel).toHaveBeenCalledTimes(1);
  });

  test('shows the busy label, disables actions, and suppresses every dismissal path', () => {
    const props = baseProps();
    render(<ConfirmDialog {...props} busy />);

    expect(screen.getByRole('button', { name: 'common.working' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'common.cancel' })).toBeDisabled();
    expect(screen.queryByRole('button', { name: 'common.close' })).not.toBeInTheDocument();

    fireEvent.keyDown(document, { key: 'Escape' });
    fireEvent.pointerDown(screen.getByRole('dialog').parentElement!);
    expect(props.onCancel).not.toHaveBeenCalled();
  });

  test('confirmDisabled disables only confirmation and preserves dismissal policy', async () => {
    const props = baseProps();
    const user = userEvent.setup();
    render(<ConfirmDialog {...props} confirmDisabled />);

    expect(screen.getByRole('button', { name: 'common.confirm' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'common.cancel' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'common.close' })).toBeInTheDocument();

    fireEvent.keyDown(document, { key: 'Escape' });
    expect(props.onCancel).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole('button', { name: 'common.cancel' }));
    expect(props.onCancel).toHaveBeenCalledTimes(2);
  });

  test('restores focus after dismissal unmounts the dialog', async () => {
    const trigger = document.createElement('button');
    document.body.appendChild(trigger);
    trigger.focus();
    const props = baseProps();
    const user = userEvent.setup();
    const { unmount } = render(<ConfirmDialog {...props} />);

    // Radix moves focus to the first focusable element inside the panel rather
    // than to the panel itself, so this asserts containment.
    expect(screen.getByRole('dialog').contains(document.activeElement)).toBe(true);
    await user.click(screen.getByRole('button', { name: 'common.cancel' }));
    unmount();

    await waitFor(() => expect(trigger).toHaveFocus());
    trigger.remove();
  });

  /**
   * The labels used to be hardcoded English defaults in the signature, which
   * `i18n:react:check` cannot see (it inspects JSX attributes, not default
   * parameter values). This pins that they resolve through `useT()` — the mock
   * above echoes the key, so a regression to a literal shows up as "Confirm"
   * rather than "common.confirm".
   */
  test('resolves its default labels through i18n rather than English literals', () => {
    render(<ConfirmDialog {...baseProps()} />);

    expect(screen.getByRole('button', { name: 'common.confirm' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'common.cancel' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Confirm' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Cancel' })).not.toBeInTheDocument();
  });

  test('still honours explicitly provided labels', () => {
    render(<ConfirmDialog {...baseProps()} confirmLabel="Delete forever" cancelLabel="Keep" />);

    expect(screen.getByRole('button', { name: 'Delete forever' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Keep' })).toBeInTheDocument();
  });

  /**
   * The three hooks below are what let a hand-rolled confirm overlay move onto
   * this component without its spec being rewritten — `FlowCanvasPage`'s two
   * inline confirms assert `flow-action-confirm` / `-cancel` /
   * `-confirm-accept` and `flow-leave-confirm` / `-stay` / `-discard`.
   */
  test('forwards testId, confirmTestId and cancelTestId to the right nodes', () => {
    render(
      <ConfirmDialog
        {...baseProps()}
        testId="flow-action-confirm"
        confirmTestId="flow-action-confirm-accept"
        cancelTestId="flow-action-cancel"
      />
    );

    expect(screen.getByTestId('flow-action-confirm')).toBe(screen.getByRole('dialog'));
    expect(screen.getByTestId('flow-action-confirm-accept')).toBe(
      screen.getByRole('button', { name: 'common.confirm' })
    );
    expect(screen.getByTestId('flow-action-cancel')).toBe(
      screen.getByRole('button', { name: 'common.cancel' })
    );
  });

  test('the forwarded hooks still drive the real callbacks', async () => {
    const props = baseProps();
    const user = userEvent.setup();
    render(
      <ConfirmDialog {...props} confirmTestId="flow-leave-discard" cancelTestId="flow-leave-stay" />
    );

    await user.click(screen.getByTestId('flow-leave-discard'));
    await user.click(screen.getByTestId('flow-leave-stay'));

    expect(props.onConfirm).toHaveBeenCalledTimes(1);
    expect(props.onCancel).toHaveBeenCalledTimes(1);
  });

  /**
   * Every existing call site passes none of the three, so the historical
   * hardcoded confirm hook has to survive and no new attribute may appear on
   * the panel or the cancel button — otherwise this is not an additive change.
   */
  test('keeps the historical confirm hook and adds nothing else when the props are omitted', () => {
    render(<ConfirmDialog {...baseProps()} />);

    expect(screen.getByTestId('confirm-dialog-confirm')).toBe(
      screen.getByRole('button', { name: 'common.confirm' })
    );
    expect(screen.getByRole('dialog')).not.toHaveAttribute('data-testid');
    expect(screen.getByRole('button', { name: 'common.cancel' })).not.toHaveAttribute(
      'data-testid'
    );
  });
});
