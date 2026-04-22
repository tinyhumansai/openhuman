import { isTauri } from '@tauri-apps/api/core';
import { useEffect, useRef, useState } from 'react';

import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

/**
 * Settings panel for voice call speech-to-text transcription.
 *
 * Controls whether OpenHuman automatically transcribes audio from embedded
 * voice calls (Slack Huddles, Discord voice channels, WhatsApp calls).
 * Requires the CEF runtime and a working Whisper model.
 */

interface CallTranscriptionSettings {
  enabled: boolean;
  providers: { slack: boolean; discord: boolean; whatsapp: boolean };
}

const DEFAULT_SETTINGS: CallTranscriptionSettings = {
  enabled: true,
  providers: { slack: true, discord: true, whatsapp: true },
};

const STORAGE_KEY = 'openhuman:call_transcription_settings';

function loadSettings(): CallTranscriptionSettings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT_SETTINGS;
    return { ...DEFAULT_SETTINGS, ...JSON.parse(raw) };
  } catch {
    return DEFAULT_SETTINGS;
  }
}

function saveSettings(s: CallTranscriptionSettings): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(s));
  } catch {
    // localStorage unavailable in some contexts — fail silently.
  }
}

const PROVIDER_LABELS: Record<string, string> = {
  slack: 'Slack Huddles',
  discord: 'Discord Voice Channels',
  whatsapp: 'WhatsApp Calls',
};

const CallTranscriptionPanel = () => {
  const { navigateBack, navigateToSettings, breadcrumbs } = useSettingsNavigation();
  const [settings, setSettings] = useState<CallTranscriptionSettings>(loadSettings);
  const [saved, setSaved] = useState(false);
  const isCef = isTauri();
  const isMounted = useRef(false);

  // Persist whenever settings change, skipping the initial mount.
  useEffect(() => {
    if (!isMounted.current) {
      isMounted.current = true;
      return;
    }
    saveSettings(settings);
    const t = setTimeout(() => setSaved(true), 0);
    const u = setTimeout(() => setSaved(false), 1500);
    return () => {
      clearTimeout(t);
      clearTimeout(u);
    };
  }, [settings]);

  const toggleGlobal = () => {
    setSettings(prev => ({ ...prev, enabled: !prev.enabled }));
  };

  const toggleProvider = (provider: keyof CallTranscriptionSettings['providers']) => {
    setSettings(prev => ({
      ...prev,
      providers: { ...prev.providers, [provider]: !prev.providers[provider] },
    }));
  };

  return (
    <div>
      <SettingsHeader
        title="Call Transcription"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4">
        {/* CEF availability notice */}
        {!isCef && (
          <div className="rounded-lg border border-amber-200 bg-amber-50 p-4 text-sm text-amber-800">
            Call transcription requires the desktop app with CEF runtime. This feature is not
            available in the browser.
          </div>
        )}

        {/* Global toggle */}
        <div className="bg-stone-50 rounded-lg border border-stone-200 p-4 space-y-2">
          <div className="flex items-center justify-between">
            <div>
              <h3 className="text-sm font-semibold text-stone-900">
                Automatically transcribe voice calls
              </h3>
              <p className="mt-1 text-xs text-stone-500">
                Record audio from embedded voice calls and transcribe using your local Whisper
                model. Transcripts are saved to memory and analysed by the AI assistant.
              </p>
            </div>
            <button
              role="switch"
              aria-checked={settings.enabled}
              onClick={toggleGlobal}
              disabled={!isCef}
              className={[
                'relative inline-flex h-6 w-11 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent',
                'transition-colors duration-200 ease-in-out focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-2',
                settings.enabled && isCef ? 'bg-primary-500' : 'bg-stone-300',
                !isCef ? 'cursor-not-allowed opacity-50' : '',
              ]
                .filter(Boolean)
                .join(' ')}>
              <span
                className={[
                  'pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow ring-0',
                  'transition duration-200 ease-in-out',
                  settings.enabled && isCef ? 'translate-x-5' : 'translate-x-0',
                ]
                  .filter(Boolean)
                  .join(' ')}
              />
            </button>
          </div>
        </div>

        {/* Per-provider toggles */}
        <div className="bg-stone-50 rounded-lg border border-stone-200">
          <div className="border-b border-stone-200 px-4 py-3">
            <h3 className="text-xs font-semibold uppercase tracking-wider text-stone-500">
              Enabled Providers
            </h3>
          </div>
          <div className="divide-y divide-stone-200">
            {(Object.keys(PROVIDER_LABELS) as (keyof CallTranscriptionSettings['providers'])[]).map(
              provider => (
                <div key={provider} className="flex items-center justify-between px-4 py-3">
                  <div>
                    <p className="text-sm font-medium text-stone-900">
                      {PROVIDER_LABELS[provider]}
                    </p>
                    <p className="mt-0.5 text-xs text-stone-400">
                      Transcribe {PROVIDER_LABELS[provider].toLowerCase()} automatically
                    </p>
                  </div>
                  <button
                    role="switch"
                    aria-checked={settings.providers[provider]}
                    onClick={() => toggleProvider(provider)}
                    disabled={!isCef || !settings.enabled}
                    className={[
                      'relative inline-flex h-5 w-9 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent',
                      'transition-colors duration-200 ease-in-out focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-2',
                      settings.providers[provider] && settings.enabled && isCef
                        ? 'bg-primary-500'
                        : 'bg-stone-300',
                      !isCef || !settings.enabled ? 'cursor-not-allowed opacity-40' : '',
                    ]
                      .filter(Boolean)
                      .join(' ')}>
                    <span
                      className={[
                        'pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow ring-0',
                        'transition duration-200 ease-in-out',
                        settings.providers[provider] && settings.enabled && isCef
                          ? 'translate-x-4'
                          : 'translate-x-0',
                      ]
                        .filter(Boolean)
                        .join(' ')}
                    />
                  </button>
                </div>
              )
            )}
          </div>
        </div>

        {/* How it works */}
        <div className="bg-stone-50 rounded-lg border border-stone-200 p-4 space-y-2">
          <h3 className="text-xs font-semibold uppercase tracking-wider text-stone-500">
            How it works
          </h3>
          <ul className="space-y-1.5 text-xs text-stone-600">
            <li className="flex gap-2">
              <span className="mt-0.5 text-primary-500">1.</span>
              <span>
                When a voice call is detected in an embedded account, audio is captured from the
                browser&apos;s audio stream via CEF.
              </span>
            </li>
            <li className="flex gap-2">
              <span className="mt-0.5 text-primary-500">2.</span>
              <span>
                When the call ends, the recorded audio is processed by your local{' '}
                <strong>Whisper</strong> speech-to-text model (no data leaves your device).
              </span>
            </li>
            <li className="flex gap-2">
              <span className="mt-0.5 text-primary-500">3.</span>
              <span>
                The transcript is saved to your memory and the AI assistant analyses it for action
                items and key points.
              </span>
            </li>
          </ul>
          <p className="mt-3 text-xs text-stone-500">
            <strong>Privacy:</strong> All processing happens locally. Audio is never sent to any
            external server. Requires a Whisper model installed via{' '}
            <button
              className="text-primary-500 underline focus:outline-none"
              onClick={() => navigateToSettings('voice')}>
              Voice Settings
            </button>
            .
          </p>
        </div>

        {/* Save confirmation */}
        {saved && (
          <p className="text-center text-xs text-sage-600 transition-opacity duration-300">
            Settings saved
          </p>
        )}
      </div>
    </div>
  );
};

export default CallTranscriptionPanel;
