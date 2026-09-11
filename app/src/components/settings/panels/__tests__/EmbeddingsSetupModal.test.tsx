/**
 * `EmbeddingsSetupModal` had no tests. It is a pure presentational component —
 * all state lives in the parent panel — so what is worth pinning is the
 * branching it owns: the custom-endpoint form vs the API-key form, and the two
 * disabled predicates on Test and Save. Those predicates are the ones a user
 * runs into (a greyed-out Save with no explanation), and they differ between
 * the two branches in ways that are easy to break.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type {
  EmbeddingProviderEntry,
  EmbeddingsTestResult,
} from '../../../../services/api/embeddingsApi';
import EmbeddingsSetupModal, { type EmbeddingsSetupModalProps } from '../EmbeddingsSetupModal';

/**
 * The mock returns the key for most lookups, which keeps assertions readable.
 * Two keys are the exception: `testSuccess` and `testFailed` are TEMPLATES that
 * the component fills with `.replace('{dims}', …)` / `.replace('{error}', …)`
 * (`EmbeddingsSetupModal.tsx:191-195`). Returning the bare key there makes
 * `.replace()` a no-op, so the substituted value never reaches the DOM and a
 * component that stopped surfacing it would keep the test green. Returning the
 * real templates (from `en.ts`) is what makes the substitution assertable.
 */
vi.mock('../../../../lib/i18n/I18nContext', () => ({
  useT: () => ({
    // Mirror the real signature: `t: (key, fallback?) => string`
    // (`lib/i18n/I18nContext.tsx:25`). Dropping the fallback made this mock
    // return the raw key for any `t(key, fallback)` call, which is not what the
    // component renders — the tooltip here is supplied as a fallback.
    t: (key: string, fallback?: string) => {
      if (key === 'settings.embeddings.testSuccess') return 'Connected: {dims} dimensions';
      if (key === 'settings.embeddings.testFailed') return 'Failed: {error}';
      return fallback ?? key;
    },
  }),
}));

const provider = (over: Partial<EmbeddingProviderEntry> = {}): EmbeddingProviderEntry => ({
  slug: 'openai',
  label: 'OpenAI',
  description: 'OpenAI embeddings',
  requires_api_key: true,
  requires_endpoint: false,
  has_api_key: false,
  models: [],
  ...over,
});

function renderModal(over: Partial<EmbeddingsSetupModalProps> = {}) {
  const props: EmbeddingsSetupModalProps = {
    setupProvider: provider(),
    onClose: vi.fn(),
    setupKey: '',
    onSetupKeyChange: vi.fn(),
    setupShowKey: false,
    onToggleShowKey: vi.fn(),
    setupTesting: false,
    setupTestResult: null,
    setupSaving: false,
    setupError: '',
    customEndpoint: '',
    onCustomEndpointChange: vi.fn(),
    customModel: '',
    onCustomModelChange: vi.fn(),
    customDims: '',
    onCustomDimsChange: vi.fn(),
    onTest: vi.fn(),
    onSave: vi.fn(),
    ...over,
  };
  render(<EmbeddingsSetupModal {...props} />);
  return props;
}

const testButton = () => screen.getByRole('button', { name: 'settings.embeddings.testConnection' });
const saveButton = () => screen.getByRole('button', { name: 'settings.embeddings.saveAndSwitch' });

describe('EmbeddingsSetupModal — API-key branch', () => {
  it('shows the provider description and hides the custom-endpoint fields', () => {
    renderModal();

    expect(screen.getByText('OpenAI embeddings')).toBeInTheDocument();
    expect(screen.queryByPlaceholderText('https://your-endpoint.com/v1')).not.toBeInTheDocument();
  });

  it('disables Test and Save while the key box is blank', () => {
    renderModal({ setupKey: '   ' });

    expect(testButton()).toBeDisabled();
    expect(saveButton()).toBeDisabled();
  });

  it('enables Test and Save once a key is typed', () => {
    renderModal({ setupKey: 'sk-live' });

    expect(testButton()).toBeEnabled();
    expect(saveButton()).toBeEnabled();
  });

  it('allows Save with no key when the provider already has one stored', () => {
    renderModal({ setupKey: '', setupProvider: provider({ has_api_key: true }) });

    // Save is about switching provider, which needs no new key...
    expect(saveButton()).toBeEnabled();
    // ...but Test sends the key in the box, so it stays disabled.
    expect(testButton()).toBeDisabled();
  });

  it('masks the key until the reveal toggle is used', () => {
    const { onToggleShowKey } = renderModal({ setupKey: 'sk-live' });

    expect(screen.getByDisplayValue('sk-live')).toHaveAttribute('type', 'password');
    fireEvent.click(screen.getByRole('button', { name: 'settings.embeddings.show' }));
    expect(onToggleShowKey).toHaveBeenCalledOnce();
  });

  it('reveals the key when the parent flips the flag', () => {
    renderModal({ setupKey: 'sk-live', setupShowKey: true });

    expect(screen.getByDisplayValue('sk-live')).toHaveAttribute('type', 'text');
    expect(screen.getByRole('button', { name: 'settings.embeddings.hide' })).toBeInTheDocument();
  });

  it('calls onTest with the typed key', () => {
    const { onTest } = renderModal({ setupKey: 'sk-live' });

    fireEvent.click(testButton());
    expect(onTest).toHaveBeenCalledOnce();
  });
});

describe('EmbeddingsSetupModal — custom-endpoint branch', () => {
  const custom = provider({ slug: 'custom', label: 'Custom', requires_endpoint: true });

  it('swaps in the endpoint, model and dimensions fields', () => {
    renderModal({ setupProvider: custom });

    expect(screen.getByPlaceholderText('https://your-endpoint.com/v1')).toBeInTheDocument();
    expect(screen.getByPlaceholderText('text-embedding-3-small')).toBeInTheDocument();
    expect(screen.getByPlaceholderText('1024')).toBeInTheDocument();
  });

  it('gates Save on the endpoint rather than on the key', () => {
    renderModal({ setupProvider: custom, customEndpoint: '  ', setupKey: 'sk-live' });
    expect(saveButton()).toBeDisabled();
  });

  it('enables Save on an endpoint alone — the key is optional here', () => {
    renderModal({ setupProvider: custom, customEndpoint: 'https://host/v1', setupKey: '' });
    expect(saveButton()).toBeEnabled();
  });

  // Was: "renders Test as enabled for a custom provider but does not call
  // onTest" — the button looked actionable and its handler opened with
  // `if (!isCustom)`, so a click produced no request, no result and no error
  // (#5909). It is now disabled instead, because `embeddings_test_connection`
  // takes only `{ provider, model, dimensions }` and cannot express a custom
  // endpoint, so there is genuinely nothing to test against.
  it('disables Test for a custom provider rather than rendering a dead control', () => {
    const { onTest } = renderModal({ setupProvider: custom, customEndpoint: 'https://host/v1' });

    expect(testButton()).toBeDisabled();
    fireEvent.click(testButton());
    expect(onTest).not.toHaveBeenCalled();
  });

  it('shows the reason as visible text, not as a title on the disabled button', () => {
    // `Button` applies `disabled:pointer-events-none` (ui/Button.tsx:40), so a
    // disabled control cannot be hovered, and a disabled button is out of the
    // tab order — a `title` there is unreachable by mouse AND by keyboard.
    // The reason is therefore rendered beside the button as ordinary text.
    renderModal({ setupProvider: custom, customEndpoint: 'https://host/v1' });

    const reason = screen.getByTestId('embeddings-test-unavailable-reason');
    expect(reason).toBeVisible();
    expect(reason.textContent ?? '').toContain('custom endpoint');
    expect(testButton()).not.toHaveAttribute('title');
  });

  it('shows no such reason for a built-in provider', () => {
    renderModal({ setupKey: 'sk-test' });

    expect(screen.queryByTestId('embeddings-test-unavailable-reason')).not.toBeInTheDocument();
  });

  it('leaves Save reachable for a custom endpoint', () => {
    // Disabling Test must not disable the path that still works: a custom
    // endpoint is saved and then checked from the Embeddings panel.
    renderModal({ setupProvider: custom, customEndpoint: 'https://host/v1' });

    expect(saveButton()).toBeEnabled();
  });
});

describe('EmbeddingsSetupModal — feedback', () => {
  const result = (over: Partial<EmbeddingsTestResult> = {}): EmbeddingsTestResult => ({
    success: true,
    provider: 'openai',
    model: 'text-embedding-3-small',
    ...over,
  });

  it('reports a successful test with the dimensions the provider returned', () => {
    renderModal({ setupTestResult: result({ actual_dimensions: 1536 }) });
    expect(screen.getByText('Connected: 1536 dimensions')).toBeInTheDocument();
  });

  it('falls back to ? when the provider reports no dimension count', () => {
    renderModal({ setupTestResult: result({ actual_dimensions: undefined }) });
    expect(screen.getByText('Connected: ? dimensions')).toBeInTheDocument();
  });

  it('reports a failed test with the provider error', () => {
    renderModal({ setupTestResult: result({ success: false, error: 'bad key' }) });
    expect(screen.getByText('Failed: bad key')).toBeInTheDocument();
  });

  it('surfaces a save error verbatim', () => {
    renderModal({ setupError: 'could not write config' });
    expect(screen.getByText('could not write config')).toBeInTheDocument();
  });

  // Testing locks the Test button only — Save stays live on purpose, so a user
  // who has already pasted a working key is not blocked behind a slow probe.
  it('locks Test but leaves Save available while a test is running', () => {
    renderModal({ setupKey: 'sk-live', setupTesting: true });

    expect(screen.getByRole('button', { name: 'settings.embeddings.testing' })).toBeDisabled();
    expect(saveButton()).toBeEnabled();
  });

  it('locks Test and Save while saving', () => {
    renderModal({ setupKey: 'sk-live', setupSaving: true });

    expect(testButton()).toBeDisabled();
    expect(screen.getByRole('button', { name: 'settings.embeddings.saving' })).toBeDisabled();
  });

  it('closes from the Cancel button', () => {
    const { onClose } = renderModal();

    fireEvent.click(screen.getByRole('button', { name: 'settings.embeddings.cancel' }));
    expect(onClose).toHaveBeenCalledOnce();
  });
});
