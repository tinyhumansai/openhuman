import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { SavingsStats, TokenjuiceSettings } from '../../../../utils/tauriCommands/tokenjuice';
import TokenUsagePanel from '../TokenUsagePanel';

/**
 * `TokenUsagePanel` is rendered by `UsagePanel` as an embedded tab, but
 * `UsagePanel.test.tsx` mocks it out (`vi.mock('../TokenUsagePanel')`), so none
 * of its 336 lines were exercised anywhere in the suite. These tests cover the
 * logic that mock hid: the three formatters, the `commitMinTokens` guards, the
 * compressor sort, and the save/error paths.
 */

const { mockGetSettings, mockGetSavings, mockUpdateSettings, mockResetSavings } = vi.hoisted(
  () => ({
    mockGetSettings: vi.fn(),
    mockGetSavings: vi.fn(),
    mockUpdateSettings: vi.fn(),
    mockResetSavings: vi.fn(),
  })
);

vi.mock('../../../../utils/tauriCommands/tokenjuice', () => ({
  getTokenjuiceSettings: () => mockGetSettings(),
  getTokenjuiceSavings: () => mockGetSavings(),
  updateTokenjuiceSettings: (p: unknown) => mockUpdateSettings(p),
  resetTokenjuiceSavings: () => mockResetSavings(),
}));

// `t` returns the key, except for the two interpolated strings, where a real
// template is returned so the `{model}` / `{count}` substitution is exercised
// rather than silently passing over a key with no placeholder in it.
const TEMPLATES: Record<string, string> = {
  'settings.tokenUsage.attributedTo': 'attributed to {model}',
  'settings.tokenUsage.overEvents': 'over {count} events',
};
vi.mock('../../../../lib/i18n/I18nContext', () => ({
  useT: () => ({ t: (k: string) => TEMPLATES[k] ?? k }),
}));

// The real `SettingsPanel` pulls in the router, the settings-route registry and
// the layout context. This panel's only decision about it is the `embedded`
// branch, so a marker stands in for the chrome.
vi.mock('../../layout/SettingsPanel', () => ({
  default: ({ children }: { children: React.ReactNode }) => (
    <div data-testid="settings-panel-chrome">{children}</div>
  ),
}));

const SETTINGS: TokenjuiceSettings = {
  router_enabled: false,
  ccr_enabled: false,
  ccr_disk_enabled: false,
  max_cache_entries: 100,
  max_cache_bytes: 1024,
  ccr_ttl_secs: null,
  min_bytes_to_compress: 512,
  ccr_min_tokens: 2000,
  search_enabled: false,
  code_enabled: false,
  html_enabled: false,
  ml_compression_enabled: false,
  ml_model_id: 'm',
  ml_target_ratio: 0.5,
  ml_sidecar_idle_timeout_secs: 60,
  ml_max_input_chars: 1000,
  ml_device: 'cpu',
};

const bucket = (over: Partial<SavingsStats['total']> = {}): SavingsStats['total'] => ({
  events: 0,
  originalTokens: 0,
  compactedTokens: 0,
  tokensSaved: 0,
  costSavedUsd: 0,
  ...over,
});

const SAVINGS: SavingsStats = {
  attributionModel: 'gpt-4.1-mini',
  total: bucket({ events: 12, tokensSaved: 34567, costSavedUsd: 12.5 }),
  byModel: {},
  byCompressor: {},
  cache: { entries: 7, bytes: 2048 },
};

function settings(over: Partial<TokenjuiceSettings> = {}): TokenjuiceSettings {
  return { ...SETTINGS, ...over };
}
function savings(over: Partial<SavingsStats> = {}): SavingsStats {
  return { ...SAVINGS, ...over };
}

/** Render and wait for the mount-time load to settle. */
async function renderPanel(props: { embedded?: boolean } = {}) {
  const utils = render(<TokenUsagePanel {...props} />);
  await waitFor(() => expect(mockGetSettings).toHaveBeenCalled());
  return utils;
}

const switchFor = (label: string) => screen.getByRole('switch', { name: label });
// `SettingsNumberField` renders `<input type="number">`, whose ARIA role is
// `spinbutton` (not `textbox`), and whose `toHaveValue` yields a number.
const minTokensInput = () =>
  screen.getByRole('spinbutton', { name: 'settings.tokenUsage.ccrMinTokens' });

beforeEach(() => {
  vi.clearAllMocks();
  mockGetSettings.mockResolvedValue(settings());
  mockGetSavings.mockResolvedValue(savings());
  mockUpdateSettings.mockImplementation((p: Partial<TokenjuiceSettings>) =>
    Promise.resolve(settings(p))
  );
  mockResetSavings.mockResolvedValue(undefined);
});

describe('TokenUsagePanel — mount load', () => {
  it('loads settings and savings once on mount', async () => {
    await renderPanel();
    await waitFor(() => expect(mockGetSavings).toHaveBeenCalled());
    expect(mockGetSettings).toHaveBeenCalledTimes(1);
    expect(mockGetSavings).toHaveBeenCalledTimes(1);
  });

  it('renders the loaded stat values', async () => {
    await renderPanel();
    // tokensSaved 34567 -> locale-grouped integer
    expect(await screen.findByText((34567).toLocaleString())).toBeInTheDocument();
    // cache.entries 7
    expect(screen.getByText('7')).toBeInTheDocument();
  });

  it('interpolates the attribution model into the section description', async () => {
    await renderPanel();
    expect(await screen.findByText('attributed to gpt-4.1-mini')).toBeInTheDocument();
  });

  it('interpolates the event count into the tokens-saved hint', async () => {
    await renderPanel();
    expect(await screen.findByText('over 12 events')).toBeInTheDocument();
  });

  it('shows em-dash placeholders and no crash when the savings load rejects', async () => {
    mockGetSettings.mockRejectedValue(new Error('core is down'));
    mockGetSavings.mockRejectedValue(new Error('core is down'));
    await renderPanel();
    await waitFor(() => expect(screen.getAllByText('—').length).toBeGreaterThan(0));
    expect(await screen.findByText(/core is down/)).toBeInTheDocument();
  });

  it('stringifies a non-Error rejection instead of rendering [object Object]', async () => {
    mockGetSettings.mockRejectedValue('plain string failure');
    mockGetSavings.mockRejectedValue('plain string failure');
    await renderPanel();
    expect(await screen.findByText(/plain string failure/)).toBeInTheDocument();
  });
});

describe('TokenUsagePanel — formatters', () => {
  it('renders a sub-cent saving as "<$0.01", not "$0.00"', async () => {
    mockGetSavings.mockResolvedValue(
      savings({ total: bucket({ events: 1, costSavedUsd: 0.004 }) })
    );
    await renderPanel();
    expect(await screen.findByText('<$0.01')).toBeInTheDocument();
    expect(screen.queryByText('$0.00')).not.toBeInTheDocument();
  });

  it('renders exactly zero as "$0.00" (the sub-cent branch is > 0 only)', async () => {
    mockGetSavings.mockResolvedValue(savings({ total: bucket({ costSavedUsd: 0 }) }));
    await renderPanel();
    expect(await screen.findByText('$0.00')).toBeInTheDocument();
    expect(screen.queryByText('<$0.01')).not.toBeInTheDocument();
  });

  it('renders a normal dollar amount to two decimal places', async () => {
    mockGetSavings.mockResolvedValue(savings({ total: bucket({ costSavedUsd: 12.5 }) }));
    await renderPanel();
    expect(await screen.findByText('$12.50')).toBeInTheDocument();
  });

  it('formats cache bytes under 1 KiB as bytes', async () => {
    mockGetSavings.mockResolvedValue(savings({ cache: { entries: 1, bytes: 512 } }));
    await renderPanel();
    expect(await screen.findByText('512 B')).toBeInTheDocument();
  });

  it('formats cache bytes under 1 MiB as KB with one decimal', async () => {
    mockGetSavings.mockResolvedValue(savings({ cache: { entries: 1, bytes: 2048 } }));
    await renderPanel();
    expect(await screen.findByText('2.0 KB')).toBeInTheDocument();
  });

  it('formats cache bytes at or over 1 MiB as MB with one decimal', async () => {
    mockGetSavings.mockResolvedValue(savings({ cache: { entries: 1, bytes: 3 * 1024 * 1024 } }));
    await renderPanel();
    expect(await screen.findByText('3.0 MB')).toBeInTheDocument();
  });
});

describe('TokenUsagePanel — byCompressor breakdown', () => {
  it('lists compressors sorted by tokensSaved, largest first', async () => {
    mockGetSavings.mockResolvedValue(
      savings({
        byCompressor: {
          small: bucket({ tokensSaved: 10, costSavedUsd: 0.5 }),
          largest: bucket({ tokensSaved: 900, costSavedUsd: 9 }),
          middle: bucket({ tokensSaved: 100, costSavedUsd: 1 }),
        },
      })
    );
    await renderPanel();
    await screen.findByText('largest');
    const names = screen.getAllByText(/^(small|largest|middle)$/).map(el => el.textContent);
    expect(names).toEqual(['largest', 'middle', 'small']);
  });

  it('omits the breakdown block entirely when byCompressor is empty', async () => {
    await renderPanel();
    expect(screen.queryByText('settings.tokenUsage.byCompressor')).not.toBeInTheDocument();
  });
});

describe('TokenUsagePanel — compression toggles', () => {
  it.each([
    ['settings.tokenUsage.routerEnabled', 'router_enabled'],
    ['settings.tokenUsage.search', 'search_enabled'],
    ['settings.tokenUsage.code', 'code_enabled'],
    ['settings.tokenUsage.html', 'html_enabled'],
    ['settings.tokenUsage.ml', 'ml_compression_enabled'],
    ['settings.tokenUsage.ccrEnabled', 'ccr_enabled'],
    ['settings.tokenUsage.ccrDisk', 'ccr_disk_enabled'],
  ])('toggling "%s" patches only %s', async (label, key) => {
    await renderPanel();
    fireEvent.click(switchFor(label));
    await waitFor(() => expect(mockUpdateSettings).toHaveBeenCalledTimes(1));
    expect(mockUpdateSettings).toHaveBeenCalledWith({ [key]: true });
  });

  it('reflects the loaded setting on the switch rather than defaulting to off', async () => {
    mockGetSettings.mockResolvedValue(settings({ router_enabled: true }));
    await renderPanel();
    await waitFor(() =>
      expect(switchFor('settings.tokenUsage.routerEnabled')).toHaveAttribute('aria-checked', 'true')
    );
  });

  it('sends false when switching an enabled setting off', async () => {
    mockGetSettings.mockResolvedValue(settings({ search_enabled: true }));
    await renderPanel();
    await waitFor(() =>
      expect(switchFor('settings.tokenUsage.search')).toHaveAttribute('aria-checked', 'true')
    );
    fireEvent.click(switchFor('settings.tokenUsage.search'));
    await waitFor(() => expect(mockUpdateSettings).toHaveBeenCalledWith({ search_enabled: false }));
  });

  it('shows the saved note after a successful patch', async () => {
    await renderPanel();
    fireEvent.click(switchFor('settings.tokenUsage.routerEnabled'));
    expect(await screen.findByText('settings.tokenUsage.saved')).toBeInTheDocument();
  });

  it('surfaces a patch failure and does not show the saved note', async () => {
    mockUpdateSettings.mockRejectedValue(new Error('write refused'));
    await renderPanel();
    fireEvent.click(switchFor('settings.tokenUsage.routerEnabled'));
    expect(await screen.findByText(/write refused/)).toBeInTheDocument();
    expect(screen.queryByText('settings.tokenUsage.saved')).not.toBeInTheDocument();
  });
});

describe('TokenUsagePanel — commitMinTokens guards', () => {
  it('patches ccr_min_tokens when the value actually changed', async () => {
    await renderPanel();
    const input = minTokensInput();
    fireEvent.change(input, { target: { value: '5000' } });
    fireEvent.blur(input);
    await waitFor(() => expect(mockUpdateSettings).toHaveBeenCalledWith({ ccr_min_tokens: 5000 }));
  });

  it('does NOT patch when the committed value equals the saved value', async () => {
    await renderPanel();
    const input = minTokensInput();
    // SETTINGS.ccr_min_tokens is 2000 and the field is seeded with it.
    fireEvent.change(input, { target: { value: '2000' } });
    fireEvent.blur(input);
    await Promise.resolve();
    expect(mockUpdateSettings).not.toHaveBeenCalled();
  });

  it('reverts to the saved value and does NOT patch on non-numeric input', async () => {
    await renderPanel();
    const input = minTokensInput();
    fireEvent.change(input, { target: { value: 'abc' } });
    fireEvent.blur(input);
    await waitFor(() => expect(input).toHaveValue(2000));
    expect(mockUpdateSettings).not.toHaveBeenCalled();
  });

  it('reverts and does NOT patch on a negative value', async () => {
    await renderPanel();
    const input = minTokensInput();
    fireEvent.change(input, { target: { value: '-5' } });
    fireEvent.blur(input);
    await waitFor(() => expect(input).toHaveValue(2000));
    expect(mockUpdateSettings).not.toHaveBeenCalled();
  });

  it('commits on Enter as well as on blur', async () => {
    await renderPanel();
    const input = minTokensInput();
    fireEvent.change(input, { target: { value: '4242' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => expect(mockUpdateSettings).toHaveBeenCalledWith({ ccr_min_tokens: 4242 }));
  });

  it('re-seeds the field from the server response after a successful patch', async () => {
    mockUpdateSettings.mockResolvedValue(settings({ ccr_min_tokens: 8888 }));
    await renderPanel();
    const input = minTokensInput();
    fireEvent.change(input, { target: { value: '5000' } });
    fireEvent.blur(input);
    // The panel trusts the server's value, not the typed one.
    await waitFor(() => expect(input).toHaveValue(8888));
  });
});

describe('TokenUsagePanel — refresh and reset', () => {
  it('refresh re-fetches savings without re-fetching settings', async () => {
    await renderPanel();
    await waitFor(() => expect(mockGetSavings).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole('button', { name: 'settings.tokenUsage.refresh' }));
    await waitFor(() => expect(mockGetSavings).toHaveBeenCalledTimes(2));
    expect(mockGetSettings).toHaveBeenCalledTimes(1);
  });

  it('reset clears savings on the core and then reloads them', async () => {
    await renderPanel();
    await waitFor(() => expect(mockGetSavings).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole('button', { name: 'settings.tokenUsage.reset' }));
    await waitFor(() => expect(mockResetSavings).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(mockGetSavings).toHaveBeenCalledTimes(2));
  });

  it('surfaces a reset failure and does not reload savings', async () => {
    mockResetSavings.mockRejectedValue(new Error('reset denied'));
    await renderPanel();
    await waitFor(() => expect(mockGetSavings).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole('button', { name: 'settings.tokenUsage.reset' }));
    expect(await screen.findByText(/reset denied/)).toBeInTheDocument();
    expect(mockGetSavings).toHaveBeenCalledTimes(1);
  });
});

describe('TokenUsagePanel — embedded chrome', () => {
  it('renders inside the SettingsPanel chrome by default', async () => {
    await renderPanel();
    expect(screen.getByTestId('settings-panel-chrome')).toBeInTheDocument();
  });

  it('renders without the chrome when embedded', async () => {
    await renderPanel({ embedded: true });
    expect(screen.queryByTestId('settings-panel-chrome')).not.toBeInTheDocument();
    // ...but the body is still there.
    expect(
      within(document.body).getByRole('switch', { name: 'settings.tokenUsage.routerEnabled' })
    ).toBeInTheDocument();
  });
});
