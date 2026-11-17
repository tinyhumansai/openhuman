import { act, fireEvent, screen } from '@testing-library/react';
import { useState } from 'react';
import { Route, Routes } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

import { I18nProvider, useT } from '../../lib/i18n/I18nContext';
import { memoryErrorMessage } from '../../services/api/memoryApi';
import { setLocale } from '../../store/localeSlice';
import { renderWithProviders } from '../../test/test-utils';
import MemoryErrorAlert from './MemoryErrorAlert';

/** Renders the alert for `err` the way the memory views do: via memoryErrorMessage. */
function Harness({ err, retry }: { err: unknown; retry?: () => void }) {
  const { t } = useT();
  return (
    <MemoryErrorAlert message={memoryErrorMessage(err, t)} data-testid="alert" onRetry={retry} />
  );
}

function renderAt(err: unknown, retry?: () => void) {
  renderWithProviders(
    <Routes>
      <Route path="/" element={<Harness err={err} retry={retry} />} />
      <Route path="/settings/account" element={<div data-testid="billing-page" />} />
    </Routes>
  );
}

const coded = (code: string, message = 'engine detail') =>
  Object.assign(new Error(message), { data: { code, kind: code } });

describe('MemoryErrorAlert', () => {
  it('turns INSUFFICIENT_CREDITS into a top-up prompt, not an error', () => {
    renderAt(coded('INSUFFICIENT_CREDITS', 'insufficient credits: HTTP 402'));
    const alert = screen.getByTestId('alert');
    expect(alert).toHaveAttribute('data-kind', 'out-of-credits');
    expect(alert).toHaveAttribute('data-variant', 'warning');
    expect(alert).toHaveTextContent('Out of credits');
    expect(alert).toHaveTextContent('nothing stored has been lost');
    expect(alert).not.toHaveTextContent('HTTP 402');
  });

  it('sends Top up to the billing page', () => {
    renderAt(coded('INSUFFICIENT_CREDITS'));
    fireEvent.click(screen.getByTestId('memory-top-up'));
    expect(screen.getByTestId('billing-page')).toBeInTheDocument();
  });

  it('turns PLAN_REQUIRED into an upgrade prompt to billing (#8081)', () => {
    renderAt(
      coded(
        'PLAN_REQUIRED',
        'plan required: [MEMORY_PLAN_REQUIRED] memory API memory/scopes (HTTP 403 Forbidden)'
      )
    );
    const alert = screen.getByTestId('alert');
    expect(alert).toHaveAttribute('data-kind', 'plan-required');
    expect(alert).toHaveAttribute('data-variant', 'warning');
    expect(alert).toHaveTextContent("Memory isn't part of your plan");
    expect(alert).toHaveTextContent("Your current plan doesn't include memory");
    expect(alert).toHaveTextContent('See plans');
    expect(alert).not.toHaveTextContent('MEMORY_PLAN_REQUIRED');
    expect(alert).not.toHaveTextContent('HTTP 403');
    expect(screen.queryByTestId('memory-top-up')).not.toBeInTheDocument();
    fireEvent.click(screen.getByTestId('memory-upgrade'));
    expect(screen.getByTestId('billing-page')).toBeInTheDocument();
  });

  it.each([
    ['UNAVAILABLE', "Memory can't be reached right now"],
    ['ENGINE', 'engine detail'],
    ['UNAUTHORIZED', 'engine detail'],
  ])('keeps the error alert for %s', (code, text) => {
    renderAt(coded(code));
    const alert = screen.getByTestId('alert');
    expect(alert).toHaveAttribute('data-variant', 'destructive');
    expect(alert).toHaveTextContent(text);
    expect(alert).not.toHaveAttribute('data-kind');
    expect(screen.queryByTestId('memory-top-up')).not.toBeInTheDocument();
  });

  it('keeps the error alert for an error with no code', () => {
    renderAt(new Error('something broke'));
    expect(screen.getByTestId('alert')).toHaveAttribute('data-variant', 'destructive');
    expect(screen.queryByTestId('memory-top-up')).not.toBeInTheDocument();
  });

  it('offers Retry beside the error for other errors', () => {
    const retry = vi.fn();
    renderAt(coded('ENGINE'), retry);
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
    expect(retry).toHaveBeenCalledTimes(1);
  });

  it('keeps Retry beside Top up when out of credits, for after a top-up', () => {
    const retry = vi.fn();
    renderAt(coded('INSUFFICIENT_CREDITS'), retry);
    expect(screen.getByTestId('memory-top-up')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
    expect(retry).toHaveBeenCalledTimes(1);
  });

  it('shows no Retry when the view offers none', () => {
    renderAt(coded('INSUFFICIENT_CREDITS'));
    expect(screen.queryByRole('button', { name: 'Try again' })).not.toBeInTheDocument();
  });

  // Accepted edge of matching on the message: views keep the text they made
  // when the action failed, so after a language switch it no longer matches
  // the new language's text and falls back to the error alert.
  it('falls back to the error alert for a message made before a language switch', () => {
    function Frozen() {
      const { t } = useT();
      const [message] = useState(() => memoryErrorMessage(coded('INSUFFICIENT_CREDITS'), t));
      return <MemoryErrorAlert message={message} data-testid="alert" />;
    }
    const { store } = renderWithProviders(
      <I18nProvider>
        <Frozen />
      </I18nProvider>
    );
    expect(screen.getByTestId('alert')).toHaveAttribute('data-kind', 'out-of-credits');

    act(() => {
      store.dispatch(setLocale('fr'));
    });
    const alert = screen.getByTestId('alert');
    expect(alert).toHaveAttribute('data-variant', 'destructive');
    expect(alert).not.toHaveAttribute('data-kind');
    expect(alert).toHaveTextContent('Top up to restore it');
    expect(screen.queryByTestId('memory-top-up')).not.toBeInTheDocument();
  });
});
