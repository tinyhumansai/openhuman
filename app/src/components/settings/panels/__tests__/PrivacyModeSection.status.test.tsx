/**
 * PrivacyModeSection — the load/save failure branches, the disabled states and
 * the auto-clearing "saved" note.
 *
 * `PrivacyModeSection.test.tsx` (sibling) covers the three options, the loaded
 * selection and the no-op re-click. Every failure branch and every transient
 * status was unexercised, which is what held branch coverage at 70.8%. This
 * matters more here than on most panels: the control sets the data-egress
 * posture (#4435), so a save that silently fails leaves the user believing they
 * are in `local_only` when they are not.
 *
 * Separate file rather than an edit to the sibling: three of us are in this
 * directory.
 */
import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import PrivacyModeSection from '../PrivacyModeSection';

const callCoreRpc = vi.fn();
vi.mock('../../../../services/coreRpcClient', () => ({
  callCoreRpc: (arg: { method: string; params: unknown }) => callCoreRpc(arg),
}));

const GET = 'openhuman.config_get_privacy_mode';
const SET = 'openhuman.config_set_privacy_mode';

/** Load resolves to `mode`; the setter behaviour is supplied per test. */
function wire(opts: {
  loadMode?: string;
  loadError?: unknown;
  onSet?: (mode?: string) => Promise<unknown>;
}) {
  callCoreRpc.mockImplementation((arg: { method: string; params: { mode?: string } }) => {
    if (arg.method === GET) {
      return opts.loadError
        ? Promise.reject(opts.loadError)
        : Promise.resolve({ result: { mode: opts.loadMode ?? 'standard' } });
    }
    if (arg.method === SET) {
      return opts.onSet
        ? opts.onSet(arg.params.mode)
        : Promise.resolve({ result: { mode: arg.params.mode } });
    }
    return Promise.reject(new Error(`unexpected method ${arg.method}`));
  });
}

const option = (mode: string) => screen.getByTestId(`privacy-mode-option-${mode}`);
const statusLine = () => document.querySelector('[data-slot="status-line"]') as HTMLElement;

beforeEach(() => {
  vi.clearAllMocks();
  // The panel logs failures through `console.warn`; keep the run readable while
  // still proving the branch executed via the rendered status line.
  vi.spyOn(console, 'warn').mockImplementation(() => {});
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe('PrivacyModeSection — load', () => {
  it('shows the load failure message and leaves nothing selected', async () => {
    wire({ loadError: new Error('privacy rpc unavailable') });
    renderWithProviders(<PrivacyModeSection />);

    expect(await screen.findByText('privacy rpc unavailable')).toBeInTheDocument();
    // `mode` stays null, so no option may claim to be the current posture.
    for (const mode of ['local_only', 'standard', 'sensitive']) {
      expect(option(mode)).not.toBeChecked();
    }
  });

  it('stringifies a non-Error load rejection rather than rendering nothing', async () => {
    wire({ loadError: 'plain string failure' });
    renderWithProviders(<PrivacyModeSection />);

    expect(await screen.findByText('plain string failure')).toBeInTheDocument();
  });

  it('disables every option while the current mode is still loading', () => {
    wire({ onSet: () => new Promise(() => {}) });
    callCoreRpc.mockImplementation(() => new Promise(() => {}));
    renderWithProviders(<PrivacyModeSection />);

    for (const mode of ['local_only', 'standard', 'sensitive']) {
      expect(option(mode)).toBeDisabled();
    }
  });

  it('enables the options once the mode has loaded', async () => {
    wire({ loadMode: 'standard' });
    renderWithProviders(<PrivacyModeSection />);

    await waitFor(() => expect(option('standard')).toBeChecked());
    expect(option('local_only')).toBeEnabled();
  });
});

describe('PrivacyModeSection — save', () => {
  it('reports a save failure and keeps the previous selection', async () => {
    wire({ loadMode: 'standard', onSet: () => Promise.reject(new Error('egress locked')) });
    renderWithProviders(<PrivacyModeSection />);
    await waitFor(() => expect(option('standard')).toBeChecked());

    fireEvent.click(option('local_only'));

    expect(await screen.findByText('egress locked')).toBeInTheDocument();
    // The posture must not appear to have changed when the write failed —
    // believing you are local-only when you are not is the whole risk here.
    expect(option('standard')).toBeChecked();
    expect(option('local_only')).not.toBeChecked();
  });

  it('adopts the mode the core echoes back, not the one that was clicked', async () => {
    // The core is authoritative: if it coerces the request, the UI must follow.
    wire({ loadMode: 'standard', onSet: () => Promise.resolve({ result: { mode: 'sensitive' } }) });
    renderWithProviders(<PrivacyModeSection />);
    await waitFor(() => expect(option('standard')).toBeChecked());

    fireEvent.click(option('local_only'));

    await waitFor(() => expect(option('sensitive')).toBeChecked());
    expect(option('local_only')).not.toBeChecked();
  });

  it('disables the options while a save is in flight', async () => {
    let release!: (v: unknown) => void;
    wire({ loadMode: 'standard', onSet: () => new Promise(resolve => (release = resolve)) });
    renderWithProviders(<PrivacyModeSection />);
    await waitFor(() => expect(option('standard')).toBeChecked());

    fireEvent.click(option('local_only'));

    await waitFor(() => expect(option('sensitive')).toBeDisabled());
    release({ result: { mode: 'local_only' } });
    await waitFor(() => expect(option('sensitive')).toBeEnabled());
  });

  it('clears the saved note after the two-second window', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    wire({ loadMode: 'standard' });
    renderWithProviders(<PrivacyModeSection />);
    await waitFor(() => expect(option('standard')).toBeChecked());

    fireEvent.click(option('local_only'));
    await waitFor(() => expect(statusLine()).toHaveTextContent(/saved/i));

    await act(async () => {
      vi.advanceTimersByTime(2000);
    });
    // `toHaveTextContent('')` is a false positive: jest-dom treats an empty
    // expected string as matching anything (it now throws rather than assert),
    // so it would pass whether or not the note cleared. `toBeEmptyDOMElement`
    // is the matcher that actually checks emptiness.
    await waitFor(() => expect(statusLine()).toBeEmptyDOMElement());
  });

  it('does not call the setter when the already-selected mode is re-chosen', async () => {
    wire({ loadMode: 'sensitive' });
    renderWithProviders(<PrivacyModeSection />);
    await waitFor(() => expect(option('sensitive')).toBeChecked());

    const before = callCoreRpc.mock.calls.filter(c => c[0].method === SET).length;
    fireEvent.click(option('sensitive'));

    expect(callCoreRpc.mock.calls.filter(c => c[0].method === SET)).toHaveLength(before);
  });
});
