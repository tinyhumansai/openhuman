/**
 * The headless message-part leaf must behave identically with and without an
 * assistant-ui runtime above it.
 *
 * That is not a nicety: `ChatThreadView` is mounted under a bare Redux
 * `Provider` by several unit tests (including the byte-frozen render-cost
 * benchmark), and assistant-ui's default client throws on a direct scope read.
 * `TextMessagePartProvider` synthesises its own `part` scope, which is why this
 * works — and this file is what stops that from silently regressing.
 */
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { AuiPlainText } from './AuiPlainText';

const CARET = <span data-testid="test-caret" />;

describe('AuiPlainText', () => {
  it('renders its text with no runtime mounted', () => {
    render(<AuiPlainText text="hello tail" />);
    expect(screen.getByText('hello tail')).toBeInTheDocument();
  });

  it('shows the in-progress affordance only while the part is running', () => {
    const { rerender } = render(<AuiPlainText text="partial" isRunning caret={CARET} />);
    expect(screen.getByTestId('test-caret')).toBeInTheDocument();

    rerender(<AuiPlainText text="partial" isRunning={false} caret={CARET} />);
    expect(screen.queryByTestId('test-caret')).not.toBeInTheDocument();
  });

  it('omits the in-progress slot entirely when no caret is supplied', () => {
    render(<AuiPlainText text="settled" isRunning />);
    expect(screen.queryByTestId('test-caret')).not.toBeInTheDocument();
    expect(screen.getByText('settled')).toBeInTheDocument();
  });
});
