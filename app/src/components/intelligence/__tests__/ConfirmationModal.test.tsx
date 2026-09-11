import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { ConfirmationModal } from '../ConfirmationModal';

describe('ConfirmationModal', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  const defaultProps = {
    modal: {
      isOpen: true,
      title: 'Test Modal',
      message: 'Test Message',
      onConfirm: vi.fn(),
      onCancel: vi.fn(),
    },
    onClose: vi.fn(),
  };

  it('renders correctly', () => {
    render(<ConfirmationModal {...defaultProps} />);
    expect(screen.getByText('Test Modal')).toBeInTheDocument();
    expect(screen.getByText('Test Message')).toBeInTheDocument();
  });

  it('calls onConfirm when confirm button is clicked', () => {
    render(<ConfirmationModal {...defaultProps} />);
    fireEvent.click(screen.getByText('Confirm'));
    expect(defaultProps.modal.onConfirm).toHaveBeenCalled();
    expect(defaultProps.onClose).toHaveBeenCalled();
  });

  it('handles dontShowAgain preference if showDontShowAgain is true', () => {
    const onConfirmMock = vi.fn();
    render(
      <ConfirmationModal
        {...defaultProps}
        modal={{
          ...defaultProps.modal,
          showDontShowAgain: true,
          preferenceKey: 'test-pref',
          onConfirm: onConfirmMock,
        }}
      />
    );

    // Toggle the checkbox
    fireEvent.click(screen.getByLabelText(/Don't show similar suggestions/i));

    // Click confirm
    fireEvent.click(screen.getByText('Confirm'));

    expect(onConfirmMock).toHaveBeenCalledWith(true);
    expect(localStorage.getItem('test-pref')).toBe('true');
  });

  it('does not store preference if preferenceKey is missing', () => {
    const onConfirmMock = vi.fn();
    render(
      <ConfirmationModal
        {...defaultProps}
        modal={{ ...defaultProps.modal, showDontShowAgain: true, onConfirm: onConfirmMock }}
      />
    );

    fireEvent.click(screen.getByLabelText(/Don't show similar suggestions/i));
    fireEvent.click(screen.getByText('Confirm'));

    expect(onConfirmMock).toHaveBeenCalledWith(true);
    expect(localStorage.length).toBe(0);
  });

  it('traps focus onto Cancel when it opens, and marks the surface a real modal', () => {
    render(<ConfirmationModal {...defaultProps} />);

    // Radix's AlertDialog auto-focuses Cancel on mount (the escape hatch) and
    // marks the content `role="alertdialog"` + `aria-modal` — none of which the
    // previous hand-rolled `fixed inset-0` div provided. Focus RESTORATION back
    // to a trigger on close is provided by the same primitive in a real
    // browser, but is not reliably observable under jsdom (no real window
    // focus), so it is not asserted here.
    const cancelButton = screen.getByText('Cancel');
    expect(document.activeElement).toBe(cancelButton);

    expect(cancelButton.closest('[role="alertdialog"]')).not.toBeNull();
  });
});
