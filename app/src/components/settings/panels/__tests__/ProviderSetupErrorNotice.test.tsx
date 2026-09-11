/**
 * `presentProviderSetupError` turns a raw provider error into the one-line
 * summary the user actually reads. It had no tests, and it is the surface a
 * user hits precisely when something has already gone wrong — a bad summary
 * here is the difference between "check your API key" and an opaque stack of
 * JSON.
 *
 * The pure function is tested directly (it is exported for exactly this
 * reason); the component is tested for the details/summary disclosure, which
 * is the only branch it owns.
 */
import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { presentProviderSetupError, ProviderSetupErrorNotice } from '../ProviderSetupErrorNotice';

vi.mock('../../../../lib/i18n/I18nContext', () => ({
  useT: () => ({ t: (key: string, fallback?: string) => fallback ?? key }),
}));

// Mirrors the `useT` contract: the component passes a fallback for every key
// that has one, so the fallback is what a user sees before translation.
const t = (key: string, fallback?: string) => fallback ?? key;

const present = (raw: string) => presentProviderSetupError(raw, t);

describe('presentProviderSetupError', () => {
  it('names the provider and blames the credentials on 401 and 403', () => {
    for (const status of [401, 403]) {
      const { summary } = present(`Could not reach OpenAI: provider returned ${status}`);
      expect(summary).toBe('OpenAI rejected the credentials. Check the API key and try again.');
    }
  });

  it('blames the base URL on 404 rather than the key', () => {
    const { summary } = present('Could not reach Anthropic: provider returned 404 Not Found');
    expect(summary).toBe(
      'Anthropic did not recognize the endpoint. Check the base URL and try again.'
    );
    expect(summary).not.toMatch(/API key/i);
  });

  it.each([500, 503, 599])('treats %s as a provider-side outage', status => {
    const { summary } = present(`Could not reach Mistral: provider returned ${status}`);
    expect(summary).toBe(
      'Mistral is unavailable right now. Try again or check the provider status.'
    );
  });

  it.each([
    'HTTP request failed',
    'error sending request for url',
    'operation timed out',
    'ECONNREFUSED 127.0.0.1:11434',
  ])('reports %s as unreachable rather than as a credential problem', cause => {
    const { summary } = present(`Could not reach Ollama: ${cause}`);
    expect(summary).toBe(
      'Could not reach Ollama. Check the endpoint URL and network connection, then try again.'
    );
  });

  it('lifts the provider message out of a JSON error body', () => {
    const { summary } = present(
      'Could not reach OpenAI: {"error":{"message":"Incorrect API key provided: sk-abc","type":"invalid_request_error"}}'
    );
    expect(summary).toBe('Could not reach OpenAI: Incorrect API key provided: sk-abc');
  });

  it('decodes escapes and collapses whitespace in a JSON message', () => {
    const { summary } = present(
      'Could not reach OpenAI: {"message":"Model \\"gpt-9\\" does not exist.\\n\\n  Try another."}'
    );
    expect(summary).toBe('Could not reach OpenAI: Model "gpt-9" does not exist. Try another.');
  });

  it('falls back to the cleaned cause when there is no status and no JSON', () => {
    const { summary } = present('Could not reach LM Studio:   something   went wrong  ');
    expect(summary).toBe('something went wrong');
  });

  // The `Could not reach X:` parse is a single-line regex — `.` does not match
  // a newline — so a multi-line provider error never yields a provider name and
  // the whole string becomes the summary. Pinned because it is surprising: the
  // same error on one line reads much better.
  it('does not strip the provider prefix from a multi-line error', () => {
    const { summary } = present('Could not reach LM Studio:   something\n\n  went wrong  ');
    expect(summary).toBe('Could not reach LM Studio: something went wrong');
  });

  it('uses a generic provider label when the error names no provider', () => {
    const { summary } = present('provider returned 401');
    expect(summary).toBe('The provider rejected the credentials. Check the API key and try again.');
  });

  it('truncates a runaway summary to 220 characters with an ellipsis', () => {
    const { summary } = present('x'.repeat(500));
    expect(summary).toHaveLength(220);
    expect(summary.endsWith('...')).toBe(true);
  });

  it('keeps a 220-character summary intact', () => {
    const { summary } = present('y'.repeat(220));
    expect(summary).toBe('y'.repeat(220));
    expect(summary.endsWith('...')).toBe(false);
  });

  it('substitutes a default for an empty error and keeps details in step', () => {
    const { summary, details } = present('   ');
    expect(details).toBe('Provider setup failed.');
    expect(summary).toBe('Provider setup failed.');
  });

  // CHARACTERISES A GAP, not desired behaviour. `presentProviderSetupError`
  // performs no redaction: `findProviderJsonMessage` lifts the provider's
  // `message` verbatim and `details` is the raw error. The backend redacts a
  // fixed set of known token prefixes, so a CUSTOM provider echoing an
  // arbitrary-shaped credential reaches the DOM intact.
  //
  // Pinned so the gap is visible and regression-locked rather than implied.
  // Redacting at this boundary is a product change — it would also hide
  // diagnostics a user may need — so it belongs in its own PR, not in a
  // test-only one. Recorded in ~/tinyhuman/bugs/W1-ui-bugs.md. When redaction
  // lands, invert these assertions.
  it('echoes an arbitrary provider credential verbatim (known gap)', () => {
    // A deliberately synthetic, self-describing stand-in — NOT a credential.
    // Its shape is the point: it matches none of the token prefixes the backend
    // redacts, which is exactly the case that slips through.
    const fakeCredential = 'FIXTURE-not-a-real-key-000000';
    const { summary, details } = present(
      `Could not reach CustomLLM: {"error":{"message":"Invalid token: ${fakeCredential}"}}`
    );

    expect(summary).toContain(fakeCredential);
    expect(details).toContain(fakeCredential);
  });

  it('preserves the raw error as details, untouched', () => {
    const raw = 'Could not reach OpenAI: provider returned 401 {"message":"nope"}';
    expect(present(raw).details).toBe(raw);
  });
});

describe('ProviderSetupErrorNotice', () => {
  it('announces itself as an alert so a screen reader reaches it', () => {
    render(<ProviderSetupErrorNotice error="Could not reach OpenAI: provider returned 401" />);
    expect(screen.getByRole('alert')).toBeInTheDocument();
  });

  it('offers the raw error behind a disclosure when it differs from the summary', () => {
    const raw = 'Could not reach OpenAI: provider returned 401';
    render(<ProviderSetupErrorNotice error={raw} />);

    expect(
      screen.getByText('OpenAI rejected the credentials. Check the API key and try again.')
    ).toBeInTheDocument();
    expect(screen.getByText(raw)).toBeInTheDocument();
  });

  it('omits the disclosure when the summary already is the whole error', () => {
    // No provider prefix, no status, no JSON: summary === details, so a
    // details block would just repeat the line above it.
    render(<ProviderSetupErrorNotice error="boom" />);

    expect(screen.getByText('boom')).toBeInTheDocument();
    expect(screen.queryByText('providerSetup.error.technicalDetails')).not.toBeInTheDocument();
  });
});
