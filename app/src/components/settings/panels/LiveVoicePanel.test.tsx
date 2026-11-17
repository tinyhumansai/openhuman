import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { LiveVoiceProviders, LiveVoiceSettings } from '../../../services/api/liveVoiceApi';
import { renderWithProviders } from '../../../test/test-utils';
import LiveVoicePanel from './LiveVoicePanel';

const api = vi.hoisted(() => ({
  fetchLiveVoiceProviders: vi.fn(),
  fetchLiveVoiceSettings: vi.fn(),
  updateLiveVoiceSettings: vi.fn(),
  testLiveVoiceProvider: vi.fn(),
  saveLiveVoiceProviderKey: vi.fn(),
  clearLiveVoiceProviderKey: vi.fn(),
}));

vi.mock('../../../services/api/liveVoiceApi', () => api);

const toastAdd = vi.hoisted(() => vi.fn());
vi.mock('../../ui/Toast', () => ({ toast: { add: toastAdd } }));

/** Wait for a toast whose fields include `fields`. */
const expectToast = (fields: Record<string, unknown>) =>
  waitFor(() => expect(toastAdd).toHaveBeenCalledWith(expect.objectContaining(fields)));

const PROVIDERS = (sarvamConfigured = false): LiveVoiceProviders => ({
  default_provider: 'gemini-hosted',
  providers: [
    {
      id: 'gemini-hosted',
      label: 'Gemini (TinyHumans)',
      kind: 'hosted',
      configured: true,
      key_slug: null,
      voices: ['Puck', 'Kore'],
      languages: ['en-US'],
    },
    {
      id: 'elevenlabs-hosted',
      label: 'ElevenLabs (TinyHumans)',
      kind: 'hosted',
      configured: true,
      key_slug: null,
      voices: [],
      languages: [],
    },
    {
      id: 'gemini',
      label: 'Gemini (own key)',
      kind: 'byok',
      configured: true,
      key_slug: 'google',
      voices: [],
      languages: [],
    },
    {
      id: 'sarvam',
      label: 'Sarvam AI',
      kind: 'byok',
      configured: sarvamConfigured,
      key_slug: 'sarvam',
      voices: ['anushka', 'abhilash'],
      languages: ['hi-IN', 'ta-IN'],
    },
  ],
});

const SETTINGS: LiveVoiceSettings = {
  default_provider: 'gemini-hosted',
  gemini: { model: null, voice: 'Puck', language: null },
  sarvam: { language: 'hi-IN', speaker: null, model: null },
  elevenlabs: { voice_id: null },
};

async function renderPanel() {
  renderWithProviders(<LiveVoicePanel />);
  await screen.findByTestId('live-voice-providers');
}

/** Open a vendor's Settings modal and return it. */
function openSettings(vendorId: string) {
  fireEvent.click(screen.getByTestId(`live-voice-settings-${vendorId}`));
  return screen.getByTestId('live-voice-modal');
}

describe('LiveVoicePanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    api.fetchLiveVoiceProviders.mockResolvedValue(PROVIDERS());
    api.fetchLiveVoiceSettings.mockResolvedValue(SETTINGS);
    api.updateLiveVoiceSettings.mockImplementation(async patch => ({ ...SETTINGS, ...patch }));
    api.saveLiveVoiceProviderKey.mockResolvedValue(undefined);
    api.clearLiveVoiceProviderKey.mockResolvedValue(undefined);
  });

  it('shows one card per vendor with the included tag and readiness', async () => {
    await renderPanel();
    expect(screen.getAllByTestId(/^live-voice-vendor-/)).toHaveLength(3);
    const gemini = screen.getByTestId('live-voice-vendor-gemini');
    expect(within(gemini).getByText('Gemini')).toBeInTheDocument();
    expect(within(gemini).getByText('Included with TinyHumans')).toBeInTheDocument();
    expect(within(gemini).getByText('In use')).toBeInTheDocument();
    expect(gemini).toHaveAttribute('data-in-use', 'true');
    // The vendor in use has no Use button; a ready one does.
    expect(screen.queryByTestId('live-voice-use-vendor-gemini')).not.toBeInTheDocument();
    expect(screen.getByTestId('live-voice-use-vendor-elevenlabs')).toBeEnabled();
    const sarvam = screen.getByTestId('live-voice-vendor-sarvam');
    expect(within(sarvam).getByText('Your own key')).toBeInTheDocument();
    expect(within(sarvam).getByText('Needs a key')).toBeInTheDocument();
    expect(screen.queryByTestId('live-voice-use-vendor-sarvam')).not.toBeInTheDocument();
    expect(screen.getByTestId('live-voice-logo-sarvam')).toBeInTheDocument();
  });

  it('switches the agent in use from a card', async () => {
    await renderPanel();
    fireEvent.click(screen.getByTestId('live-voice-use-vendor-elevenlabs'));
    await waitFor(() =>
      expect(api.updateLiveVoiceSettings).toHaveBeenCalledWith({
        default_provider: 'elevenlabs-hosted',
      })
    );
    await expectToast({ type: 'success', title: 'Voice agent switched' });
    expect(screen.getByTestId('live-voice-vendor-elevenlabs')).toHaveAttribute(
      'data-in-use',
      'true'
    );
    expect(screen.getByTestId('live-voice-vendor-gemini')).not.toHaveAttribute('data-in-use');
  });

  it('lists managed before own-key options in the settings modal and switches between them', async () => {
    await renderPanel();
    const modal = openSettings('gemini');
    const options = within(modal).getAllByTestId(/^live-voice-option-/);
    expect(options.map(o => o.dataset.testid)).toEqual([
      'live-voice-option-gemini-hosted',
      'live-voice-option-gemini',
    ]);
    expect(within(options[0]).getByText('Managed by TinyHumans')).toBeInTheDocument();
    expect(within(options[0]).getByText('Included with TinyHumans')).toBeInTheDocument();
    expect(within(modal).getByTestId('live-voice-in-use-gemini-hosted')).toBeInTheDocument();

    fireEvent.click(within(modal).getByTestId('live-voice-use-gemini'));
    await waitFor(() =>
      expect(api.updateLiveVoiceSettings).toHaveBeenCalledWith({ default_provider: 'gemini' })
    );
    await within(modal).findByTestId('live-voice-in-use-gemini');
    await expectToast({ type: 'success', title: 'Voice agent switched' });

    fireEvent.click(within(modal).getByTestId('live-voice-modal-close'));
    expect(screen.queryByTestId('live-voice-modal')).not.toBeInTheDocument();
  });

  it('saves a BYOK key from the modal and re-fetches providers', async () => {
    await renderPanel();
    const modal = openSettings('sarvam');
    expect(within(modal).getByText('Add an API key to use this agent.')).toBeInTheDocument();
    // No Use or Test for an option without a key.
    expect(within(modal).queryByTestId('live-voice-use-sarvam')).not.toBeInTheDocument();
    expect(within(modal).queryByTestId('live-voice-test-button-sarvam')).not.toBeInTheDocument();
    const save = within(modal).getByTestId('live-voice-save-key-sarvam');
    expect(save).toBeDisabled();
    api.fetchLiveVoiceProviders.mockResolvedValueOnce(PROVIDERS(true));

    fireEvent.change(within(modal).getByTestId('live-voice-key-sarvam'), {
      target: { value: 'sk-sarvam' },
    });
    fireEvent.click(save);

    await waitFor(() =>
      expect(api.saveLiveVoiceProviderKey).toHaveBeenCalledWith('sarvam', 'sk-sarvam')
    );
    await expectToast({ type: 'success', title: 'API key added' });
    expect(api.fetchLiveVoiceProviders).toHaveBeenCalledTimes(2);
    // Once stored, the key collapses to a summary and the option becomes usable.
    expect(within(modal).getByText('API key saved')).toBeInTheDocument();
    expect(within(modal).queryByTestId('live-voice-key-sarvam')).not.toBeInTheDocument();
    expect(within(modal).getByTestId('live-voice-use-sarvam')).toBeEnabled();
  });

  it('opens and cancels the replace-key editor for a stored key', async () => {
    await renderPanel();
    const modal = openSettings('gemini');
    const toggle = within(modal).getByTestId('live-voice-replace-key-gemini');
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute('aria-expanded', 'true');
    expect(within(modal).getByTestId('live-voice-key-gemini')).toBeInTheDocument();
    fireEvent.click(toggle);
    expect(within(modal).queryByTestId('live-voice-key-gemini')).not.toBeInTheDocument();
  });

  it('removes a stored key', async () => {
    await renderPanel();
    const modal = openSettings('gemini');
    fireEvent.click(within(modal).getByTestId('live-voice-clear-key-gemini'));
    await waitFor(() => expect(api.clearLiveVoiceProviderKey).toHaveBeenCalledWith('google'));
    await expectToast({ type: 'success', title: 'API key removed' });
  });

  it('surfaces a key-save failure', async () => {
    api.saveLiveVoiceProviderKey.mockRejectedValueOnce(new Error('keyring locked'));
    await renderPanel();
    const modal = openSettings('gemini');
    fireEvent.click(within(modal).getByTestId('live-voice-replace-key-gemini'));
    fireEvent.change(within(modal).getByTestId('live-voice-key-gemini'), {
      target: { value: 'k' },
    });
    fireEvent.click(within(modal).getByTestId('live-voice-save-key-gemini'));
    await expectToast({ type: 'error', description: 'keyring locked' });
  });

  it('surfaces a key-removal failure', async () => {
    api.clearLiveVoiceProviderKey.mockRejectedValueOnce(new Error('nope'));
    await renderPanel();
    const modal = openSettings('gemini');
    fireEvent.click(within(modal).getByTestId('live-voice-clear-key-gemini'));
    await expectToast({ type: 'error', description: 'nope' });
  });

  it('runs a provider test and shows latency, then a failure', async () => {
    api.testLiveVoiceProvider.mockResolvedValueOnce({ ok: true, latency_ms: 240, error: null });
    await renderPanel();
    let modal = openSettings('gemini');
    fireEvent.click(within(modal).getByTestId('live-voice-test-button-gemini-hosted'));
    expect(api.testLiveVoiceProvider).toHaveBeenCalledWith('gemini-hosted');
    const line = await within(modal).findByText('Working · 240 ms');
    expect(line).toHaveAttribute('data-ok', 'true');

    api.testLiveVoiceProvider.mockResolvedValueOnce({
      ok: false,
      latency_ms: null,
      error: 'bad key',
    });
    fireEvent.click(within(modal).getByTestId('live-voice-test-button-gemini'));
    await within(modal).findByText('Test failed: bad key');

    api.testLiveVoiceProvider.mockRejectedValueOnce(new Error('timeout'));
    fireEvent.click(within(modal).getByTestId('live-voice-test-button-gemini-hosted'));
    await within(modal).findByText('Test failed: timeout');

    fireEvent.click(within(modal).getByTestId('live-voice-modal-close'));
    api.testLiveVoiceProvider.mockResolvedValueOnce({ ok: true, latency_ms: null, error: null });
    modal = openSettings('elevenlabs');
    fireEvent.click(within(modal).getByTestId('live-voice-test-button-elevenlabs-hosted'));
    await within(modal).findByText('Working');
  });

  it('shows a testing state while the probe is in flight', async () => {
    let finish: (v: unknown) => void = () => undefined;
    api.testLiveVoiceProvider.mockReturnValueOnce(new Promise(r => (finish = r)));
    await renderPanel();
    const modal = openSettings('gemini');
    fireEvent.click(within(modal).getByTestId('live-voice-test-button-gemini'));
    expect(within(modal).getByTestId('live-voice-test-button-gemini')).toBeDisabled();
    expect(within(modal).getByTestId('live-voice-test-gemini')).toHaveTextContent('Testing…');
    finish({ ok: true, latency_ms: 5, error: null });
    await within(modal).findByText('Working · 5 ms');
  });

  it('writes Gemini voice and language to the shared gemini block', async () => {
    await renderPanel();
    const modal = openSettings('gemini');
    expect((within(modal).getByTestId('live-voice-voice-gemini') as HTMLSelectElement).value).toBe(
      'Puck'
    );
    fireEvent.change(within(modal).getByTestId('live-voice-language-gemini'), {
      target: { value: 'en-US' },
    });
    await waitFor(() =>
      expect(api.updateLiveVoiceSettings).toHaveBeenCalledWith({ gemini: { language: 'en-US' } })
    );
  });

  it('writes Sarvam speaker and language to the sarvam block', async () => {
    await renderPanel();
    const modal = openSettings('sarvam');
    expect(within(modal).getByText('Speaker')).toBeInTheDocument();
    expect(
      (within(modal).getByTestId('live-voice-language-sarvam') as HTMLSelectElement).value
    ).toBe('hi-IN');
    fireEvent.change(within(modal).getByTestId('live-voice-voice-sarvam'), {
      target: { value: 'anushka' },
    });
    await waitFor(() =>
      expect(api.updateLiveVoiceSettings).toHaveBeenCalledWith({ sarvam: { speaker: 'anushka' } })
    );
    fireEvent.change(within(modal).getByTestId('live-voice-language-sarvam'), {
      target: { value: '' },
    });
    await waitFor(() =>
      expect(api.updateLiveVoiceSettings).toHaveBeenCalledWith({ sarvam: { language: null } })
    );
  });

  it('hides voice settings for a vendor that lists no voices or languages', async () => {
    await renderPanel();
    const modal = openSettings('elevenlabs');
    expect(within(modal).queryByText('Voice settings')).not.toBeInTheDocument();
  });

  it('reports a save error as a toast', async () => {
    api.updateLiveVoiceSettings.mockRejectedValueOnce(new Error('disk full'));
    await renderPanel();
    fireEvent.click(screen.getByTestId('live-voice-use-vendor-elevenlabs'));
    await expectToast({
      type: 'error',
      title: "Couldn't save your changes",
      description: 'disk full',
    });
  });

  it('toasts a switch and a new key with the vendor named and its logo', async () => {
    await renderPanel();
    fireEvent.click(screen.getByTestId('live-voice-use-vendor-elevenlabs'));
    await expectToast({
      type: 'success',
      title: 'Voice agent switched',
      description: 'ElevenLabs now answers when you talk to your assistant.',
    });
    expect(toastAdd.mock.lastCall?.[0].data.icon).toBeTruthy();

    const modal = openSettings('sarvam');
    fireEvent.change(within(modal).getByTestId('live-voice-language-sarvam'), {
      target: { value: 'ta-IN' },
    });
    await expectToast({ type: 'success', title: 'Voice settings updated' });

    api.fetchLiveVoiceProviders.mockResolvedValueOnce(PROVIDERS(true));
    fireEvent.change(within(modal).getByTestId('live-voice-key-sarvam'), {
      target: { value: 'sk' },
    });
    fireEvent.click(within(modal).getByTestId('live-voice-save-key-sarvam'));
    await expectToast({
      type: 'success',
      title: 'API key added',
      description: 'Sarvam AI is ready to use.',
    });
  });

  it('shows a load error', async () => {
    api.fetchLiveVoiceProviders.mockRejectedValueOnce(new Error('core down'));
    renderWithProviders(<LiveVoicePanel />);
    await screen.findByText(/Couldn't load voice agent settings: core down/);
    expect(screen.queryByTestId('live-voice-providers')).not.toBeInTheDocument();
  });
});
