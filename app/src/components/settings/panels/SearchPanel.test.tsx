/**
 * Tests for SearchPanel — the "Allowed websites" (unified web-access firewall)
 * section.
 *
 * Covers the tri-state access mode (Allow all / Custom / Block all):
 *  - deriving the initial mode from the loaded settings,
 *  - "Allow all"  → persists `allow_all: true`,
 *  - "Block all"  → persists `allowed_domains: []` + `allow_all: false`,
 *  - "Custom"     → reveals the host editor and saving persists the list.
 */
import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import SearchPanel from './SearchPanel';

// ---------------------------------------------------------------------------
// Hoisted mocks
// ---------------------------------------------------------------------------
const hoisted = vi.hoisted(() => ({ getSearchSettings: vi.fn(), updateSearchSettings: vi.fn() }));

vi.mock('../../../utils/tauriCommands/config', () => ({
  openhumanGetSearchSettings: (...a: unknown[]) => hoisted.getSearchSettings(...a),
  openhumanUpdateSearchSettings: (...a: unknown[]) => hoisted.updateSearchSettings(...a),
}));

// Identity translator so we can query by the stable i18n keys.
vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));

// Authed (non-local) session so the panel behaves normally.
vi.mock('../../../utils/localSession', () => ({ isLocalSessionToken: () => false }));

function settings(overrides: Record<string, unknown> = {}) {
  return {
    engine: 'managed',
    effective_engine: 'managed',
    max_results: 5,
    timeout_secs: 15,
    parallel_configured: false,
    brave_configured: false,
    querit_configured: false,
    allowed_domains: ['reuters.com'],
    allow_all: false,
    ...overrides,
  };
}

const PLACEHOLDER = 'settings.search.allowedSitesPlaceholder';
const ALLOW_ALL = 'settings.search.accessAllowAll';
const CUSTOM = 'settings.search.accessCustom';
const BLOCK_ALL = 'settings.search.accessBlockAll';

const radio = (name: string) => screen.getByRole('radio', { name });
const keyEditor = (label: string) => within(screen.getByRole('group', { name: label }));

describe('SearchPanel — unified web-access modes', () => {
  beforeEach(() => {
    hoisted.getSearchSettings.mockReset();
    hoisted.updateSearchSettings.mockReset();
    hoisted.getSearchSettings.mockResolvedValue({ result: settings() });
    hoisted.updateSearchSettings.mockResolvedValue({ result: {} });
  });

  test('explicit host list → starts in Custom mode with the editor populated', async () => {
    renderWithProviders(<SearchPanel embedded />);
    // The textarea mounts empty, then a one-time sync effect fills it from
    // settings on the next tick — wait for the value rather than asserting now.
    await waitFor(() => {
      const ta = screen.getByPlaceholderText(PLACEHOLDER) as HTMLTextAreaElement;
      expect(ta.value).toBe('reuters.com');
    });
    expect(radio(CUSTOM)).toHaveAttribute('aria-checked', 'true');
    expect(radio(ALLOW_ALL)).toHaveAttribute('aria-checked', 'false');
  });

  test('selecting Disabled persists the disabled engine', async () => {
    renderWithProviders(<SearchPanel embedded />);
    const disabled = await screen.findByTestId('search-engine-disabled');

    fireEvent.click(disabled);

    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({ engine: 'disabled' })
    );
  });

  test('disabled settings start with Disabled selected and no needs-key badge', async () => {
    hoisted.getSearchSettings.mockResolvedValue({
      result: settings({ engine: 'disabled', effective_engine: 'disabled' }),
    });
    renderWithProviders(<SearchPanel embedded />);

    const disabled = await screen.findByTestId('search-engine-disabled');

    expect(disabled).toHaveAttribute('aria-checked', 'true');
    expect(within(disabled).queryByText('settings.search.statusNeedsKey')).toBeNull();
  });

  test('selecting "Allow all" persists allow_all: true and hides the editor', async () => {
    renderWithProviders(<SearchPanel embedded />);
    await screen.findByPlaceholderText(PLACEHOLDER);

    fireEvent.click(radio(ALLOW_ALL));

    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({ allow_all: true })
    );
    expect(screen.queryByPlaceholderText(PLACEHOLDER)).toBeNull();
  });

  test('selecting "Block all" persists an empty allowlist and hides the editor', async () => {
    renderWithProviders(<SearchPanel embedded />);
    await screen.findByPlaceholderText(PLACEHOLDER);

    fireEvent.click(radio(BLOCK_ALL));

    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        allowed_domains: [],
        allow_all: false,
      })
    );
    expect(screen.queryByPlaceholderText(PLACEHOLDER)).toBeNull();
  });

  test('Custom: saving an edited host list persists allowed_domains + allow_all: false', async () => {
    renderWithProviders(<SearchPanel embedded />);
    const textarea = await screen.findByPlaceholderText(PLACEHOLDER);

    fireEvent.change(textarea, { target: { value: 'github.com\n  apnews.com  \n\n' } });
    fireEvent.click(screen.getByText('settings.search.allowedSitesSave'));

    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        allowed_domains: ['github.com', 'apnews.com'],
        allow_all: false,
      })
    );
  });

  test('Custom: pasted URLs are normalized to bare hosts before persisting', async () => {
    renderWithProviders(<SearchPanel embedded />);
    const textarea = await screen.findByPlaceholderText(PLACEHOLDER);

    // Users paste full URLs; url_guard matches on host, so a scheme/path entry
    // would never match. The editor strips both down to the bare host.
    fireEvent.change(textarea, {
      target: { value: 'https://reuters.com/markets\nhttp://apnews.com/\ngithub.com' },
    });
    fireEvent.click(screen.getByText('settings.search.allowedSitesSave'));

    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        allowed_domains: ['reuters.com', 'apnews.com', 'github.com'],
        allow_all: false,
      })
    );
  });

  test('allow_all settings → starts in Allow-all mode with no editor', async () => {
    hoisted.getSearchSettings.mockResolvedValue({
      result: settings({ allowed_domains: ['*'], allow_all: true }),
    });
    renderWithProviders(<SearchPanel embedded />);

    await waitFor(() => expect(radio(ALLOW_ALL)).toHaveAttribute('aria-checked', 'true'));
    expect(screen.queryByPlaceholderText(PLACEHOLDER)).toBeNull();
  });

  test('empty allowlist → starts in Block-all mode with no editor', async () => {
    hoisted.getSearchSettings.mockResolvedValue({
      result: settings({ allowed_domains: [], allow_all: false }),
    });
    renderWithProviders(<SearchPanel embedded />);

    await waitFor(() => expect(radio(BLOCK_ALL)).toHaveAttribute('aria-checked', 'true'));
    expect(screen.queryByPlaceholderText(PLACEHOLDER)).toBeNull();
  });

  test('switching Block → Custom keeps the previously typed hosts', async () => {
    renderWithProviders(<SearchPanel embedded />);
    const textarea = (await screen.findByPlaceholderText(PLACEHOLDER)) as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: 'example.com' } });

    // Block all (persists empty list) then back to Custom — the editor text is
    // local state and must survive the round trip so the user doesn't lose it.
    fireEvent.click(radio(BLOCK_ALL));
    await waitFor(() => expect(screen.queryByPlaceholderText(PLACEHOLDER)).toBeNull());
    fireEvent.click(radio(CUSTOM));

    const reopened = (await screen.findByPlaceholderText(PLACEHOLDER)) as HTMLTextAreaElement;
    expect(reopened.value).toBe('example.com');
  });

  test('saving Parallel and Brave API keys sends provider-specific patches', async () => {
    renderWithProviders(<SearchPanel embedded />);
    await screen.findByPlaceholderText('settings.search.placeholderParallel');

    const parallel = keyEditor('settings.search.parallelKeyLabel');
    const parallelInput = parallel.getByPlaceholderText(
      'settings.search.placeholderParallel'
    ) as HTMLInputElement;
    fireEvent.change(parallelInput, { target: { value: 'parallel-test-key' } });
    fireEvent.click(parallel.getByText('settings.search.save'));

    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        parallel_api_key: 'parallel-test-key',
      })
    );
    expect(parallelInput.value).toBe('');

    const brave = keyEditor('settings.search.braveKeyLabel');
    const braveInput = brave.getByPlaceholderText(
      'settings.search.placeholderBrave'
    ) as HTMLInputElement;
    fireEvent.change(braveInput, { target: { value: 'brave-test-key' } });
    fireEvent.click(brave.getByText('settings.search.save'));

    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({ brave_api_key: 'brave-test-key' })
    );
    expect(braveInput.value).toBe('');
  });

  test('Parallel and Brave key editors can reveal and clear stored keys', async () => {
    hoisted.getSearchSettings.mockResolvedValue({
      result: settings({ parallel_configured: true, brave_configured: true }),
    });
    renderWithProviders(<SearchPanel embedded />);
    await screen.findAllByPlaceholderText('settings.search.placeholderStored');

    const parallel = keyEditor('settings.search.parallelKeyLabel');
    const parallelInput = parallel.getByPlaceholderText(
      'settings.search.placeholderStored'
    ) as HTMLInputElement;
    expect(parallelInput.type).toBe('password');

    fireEvent.click(parallel.getByText('settings.search.show'));
    expect(parallelInput.type).toBe('text');
    fireEvent.click(parallel.getByText('settings.search.clear'));

    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({ parallel_api_key: '' })
    );

    const brave = keyEditor('settings.search.braveKeyLabel');
    const braveInput = brave.getByPlaceholderText(
      'settings.search.placeholderStored'
    ) as HTMLInputElement;
    expect(braveInput.type).toBe('password');

    fireEvent.click(brave.getByText('settings.search.show'));
    expect(braveInput.type).toBe('text');
    fireEvent.click(brave.getByText('settings.search.clear'));

    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({ brave_api_key: '' })
    );
  });

  test('Querit key editor can reveal, save, and clear the stored API key', async () => {
    hoisted.getSearchSettings.mockResolvedValue({ result: settings({ querit_configured: true }) });
    renderWithProviders(<SearchPanel embedded />);
    await screen.findByPlaceholderText('settings.search.placeholderStored');

    const querit = keyEditor('settings.search.queritKeyLabel');
    const input = querit.getByPlaceholderText(
      'settings.search.placeholderStored'
    ) as HTMLInputElement;
    expect(input.type).toBe('password');

    fireEvent.click(querit.getByText('settings.search.show'));
    expect(input.type).toBe('text');

    fireEvent.change(input, { target: { value: 'querit-test-key' } });
    fireEvent.click(querit.getByText('settings.search.save'));

    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        querit_api_key: 'querit-test-key',
      })
    );
    expect(input.value).toBe('');

    fireEvent.click(querit.getByText('settings.search.clear'));

    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({ querit_api_key: '' })
    );
  });
});
