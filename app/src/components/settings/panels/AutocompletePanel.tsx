import { useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  type AutocompleteConfig,
  type AutocompleteStatus,
  isTauri,
  openhumanAutocompleteSetStyle,
  openhumanAutocompleteStart,
  openhumanAutocompleteStatus,
  openhumanAutocompleteStop,
  openhumanGetConfig,
} from '../../../utils/tauriCommands';
import Button from '../../ui/Button';
import {
  SettingsNumberField,
  SettingsRow,
  SettingsSection,
  SettingsSelect,
  SettingsStatusLine,
  SettingsSwitch,
  SettingsTextArea,
} from '../controls';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import SettingsPanel from '../layout/SettingsPanel';

const DEFAULT_CONFIG: AutocompleteConfig = {
  enabled: false,
  debounce_ms: 120,
  max_chars: 384,
  style_preset: 'balanced',
  style_instructions: null,
  style_examples: [],
  disabled_apps: [],
  accept_with_tab: true,
  overlay_ttl_ms: 1100,
};

const parseAutocompleteConfig = (raw: unknown): AutocompleteConfig => {
  if (!raw || typeof raw !== 'object') {
    return DEFAULT_CONFIG;
  }
  const value = raw as Record<string, unknown>;
  return {
    enabled: typeof value.enabled === 'boolean' ? value.enabled : DEFAULT_CONFIG.enabled,
    debounce_ms:
      typeof value.debounce_ms === 'number' ? value.debounce_ms : DEFAULT_CONFIG.debounce_ms,
    max_chars: typeof value.max_chars === 'number' ? value.max_chars : DEFAULT_CONFIG.max_chars,
    style_preset:
      typeof value.style_preset === 'string' ? value.style_preset : DEFAULT_CONFIG.style_preset,
    style_instructions:
      typeof value.style_instructions === 'string' ? value.style_instructions : null,
    style_examples: Array.isArray(value.style_examples)
      ? value.style_examples.filter((entry): entry is string => typeof entry === 'string')
      : DEFAULT_CONFIG.style_examples,
    disabled_apps: Array.isArray(value.disabled_apps)
      ? value.disabled_apps.filter((entry): entry is string => typeof entry === 'string')
      : DEFAULT_CONFIG.disabled_apps,
    accept_with_tab:
      typeof value.accept_with_tab === 'boolean'
        ? value.accept_with_tab
        : DEFAULT_CONFIG.accept_with_tab,
    overlay_ttl_ms:
      typeof value.overlay_ttl_ms === 'number'
        ? value.overlay_ttl_ms
        : DEFAULT_CONFIG.overlay_ttl_ms,
  };
};

const AutocompletePanel = () => {
  const { t } = useT();
  const { navigateToSettings } = useSettingsNavigation();
  const [status, setStatus] = useState<AutocompleteStatus | null>(null);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  const [enabled, setEnabled] = useState<boolean>(DEFAULT_CONFIG.enabled);
  const [stylePreset, setStylePreset] = useState<string>(DEFAULT_CONFIG.style_preset);
  const [disabledAppsText, setDisabledAppsText] = useState<string>(
    DEFAULT_CONFIG.disabled_apps.join('\n')
  );
  const [acceptWithTab, setAcceptWithTab] = useState<boolean>(DEFAULT_CONFIG.accept_with_tab);
  // Tuning fields are kept as raw input strings so the user can clear a field
  // mid-edit (e.g. 800 → "" → 512) without it snapping to the minimum on every
  // keystroke; they are parsed and clamped to safe values at save time.
  const [debounceMs, setDebounceMs] = useState<string>(String(DEFAULT_CONFIG.debounce_ms));
  const [maxChars, setMaxChars] = useState<string>(String(DEFAULT_CONFIG.max_chars));
  const [overlayTtlMs, setOverlayTtlMs] = useState<string>(String(DEFAULT_CONFIG.overlay_ttl_ms));

  const fullConfigRef = useRef<AutocompleteConfig>(DEFAULT_CONFIG);
  const [configLoaded, setConfigLoaded] = useState(false);

  const load = async () => {
    if (!isTauri()) return;
    setError(null);
    try {
      const [statusResponse, configResponse] = await Promise.all([
        openhumanAutocompleteStatus(),
        openhumanGetConfig(),
      ]);
      setStatus(statusResponse.result);
      const config = parseAutocompleteConfig(
        (configResponse.result.config as Record<string, unknown> | undefined)?.autocomplete
      );
      fullConfigRef.current = config;
      setConfigLoaded(true);
      setEnabled(config.enabled);
      setStylePreset(config.style_preset);
      setDisabledAppsText(config.disabled_apps.join('\n'));
      setAcceptWithTab(config.accept_with_tab);
      setDebounceMs(String(config.debounce_ms));
      setMaxChars(String(config.max_chars));
      setOverlayTtlMs(String(config.overlay_ttl_ms));
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load autocomplete settings');
    }
  };

  useEffect(() => {
    void load();
  }, []);

  const refreshStatus = async () => {
    if (!isTauri()) return;
    try {
      const response = await openhumanAutocompleteStatus();
      setStatus(response.result);
    } catch {
      // Non-critical
    }
  };

  useEffect(() => {
    if (!isTauri()) return;
    const intervalId = window.setInterval(() => {
      void refreshStatus();
    }, 1200);
    return () => window.clearInterval(intervalId);
  }, []);

  const saveConfig = async () => {
    if (!isTauri()) return;
    setIsSaving(true);
    setError(null);
    setMessage(null);
    try {
      const prev = fullConfigRef.current;
      // Parse + clamp the raw input strings only at save time so intermediate
      // empty/partial values are allowed while typing.
      const parsedDebounce = Math.max(0, Math.trunc(Number(debounceMs)) || 0);
      const parsedMaxChars = Math.max(1, Math.trunc(Number(maxChars)) || DEFAULT_CONFIG.max_chars);
      const parsedOverlayTtl = Math.max(0, Math.trunc(Number(overlayTtlMs)) || 0);
      const response = await openhumanAutocompleteSetStyle({
        enabled,
        debounce_ms: parsedDebounce,
        max_chars: parsedMaxChars,
        style_preset: stylePreset.trim() || 'balanced',
        style_instructions: prev.style_instructions ?? undefined,
        style_examples: prev.style_examples,
        disabled_apps: disabledAppsText
          .split('\n')
          .map(entry => entry.trim())
          .filter(Boolean),
        accept_with_tab: acceptWithTab,
        overlay_ttl_ms: parsedOverlayTtl,
      });

      fullConfigRef.current = response.result.config;
      setEnabled(response.result.config.enabled);
      setStylePreset(response.result.config.style_preset);
      setDisabledAppsText(response.result.config.disabled_apps.join('\n'));
      setAcceptWithTab(response.result.config.accept_with_tab);
      setDebounceMs(String(response.result.config.debounce_ms));
      setMaxChars(String(response.result.config.max_chars));
      setOverlayTtlMs(String(response.result.config.overlay_ttl_ms));
      setMessage(t('autocomplete.settingsSaved'));
      await refreshStatus();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save autocomplete settings');
    } finally {
      setIsSaving(false);
    }
  };

  const start = async () => {
    if (!isTauri()) return;
    setError(null);
    setMessage(null);
    try {
      const response = await openhumanAutocompleteStart({
        debounce_ms: fullConfigRef.current.debounce_ms,
      });
      await refreshStatus();
      if (response.result.started) {
        setMessage(t('autocomplete.started'));
      } else {
        setMessage(t('autocomplete.didNotStart'));
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to start autocomplete');
    }
  };

  const stop = async () => {
    if (!isTauri()) return;
    setError(null);
    setMessage(null);
    try {
      await openhumanAutocompleteStop({ reason: 'manual_stop_from_settings' });
      await refreshStatus();
      setMessage(t('autocomplete.stopped'));
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to stop autocomplete');
    }
  };

  return (
    <SettingsPanel>
      {/* ── Settings ──────────────────────────────────────────────── */}
      <SettingsSection title={t('autocomplete.settings')}>
        <SettingsRow
          label={t('common.enabled')}
          control={
            <SettingsSwitch
              id="autocomplete-enabled"
              checked={enabled}
              onCheckedChange={setEnabled}
              aria-label={t('common.enabled')}
            />
          }
        />

        <SettingsRow
          label={t('autocomplete.acceptWithTab')}
          control={
            <SettingsSwitch
              id="autocomplete-accept-with-tab"
              checked={acceptWithTab}
              onCheckedChange={setAcceptWithTab}
              aria-label={t('autocomplete.acceptWithTab')}
            />
          }
        />

        <SettingsRow
          htmlFor="autocomplete-style-preset"
          label={t('autocomplete.stylePreset')}
          control={
            <SettingsSelect
              id="autocomplete-style-preset"
              value={stylePreset}
              onChange={e => setStylePreset(e.target.value)}>
              <option value="balanced">{t('autocomplete.style.balanced')}</option>
              <option value="concise">{t('autocomplete.style.concise')}</option>
              <option value="formal">{t('autocomplete.style.formal')}</option>
              <option value="casual">{t('autocomplete.style.casual')}</option>
              <option value="custom">{t('autocomplete.style.custom')}</option>
            </SettingsSelect>
          }
        />

        <SettingsRow
          htmlFor="autocomplete-disabled-apps"
          label={t('autocomplete.disabledApps')}
          stacked
          control={
            <SettingsTextArea
              id="autocomplete-disabled-apps"
              value={disabledAppsText}
              onChange={e => setDisabledAppsText(e.target.value)}
              rows={3}
            />
          }
        />

        <SettingsRow
          label={t('autocomplete.debounceMs')}
          control={
            <SettingsNumberField
              id="autocomplete-debounce-ms-field"
              value={debounceMs}
              onChange={setDebounceMs}
              onCommit={() => void saveConfig()}
              unit="ms"
              min={0}
              max={5000}
              aria-label={t('autocomplete.debounceMs')}
              data-testid="autocomplete-debounce-ms"
            />
          }
        />

        <SettingsRow
          label={t('autocomplete.maxChars')}
          control={
            <SettingsNumberField
              id="autocomplete-max-chars-field"
              value={maxChars}
              onChange={setMaxChars}
              onCommit={() => void saveConfig()}
              unit={t('autocomplete.chars')}
              min={1}
              max={8192}
              aria-label={t('autocomplete.maxChars')}
              data-testid="autocomplete-max-chars"
            />
          }
        />

        <SettingsRow
          label={t('autocomplete.overlayTtlMs')}
          control={
            <SettingsNumberField
              id="autocomplete-overlay-ttl-ms-field"
              value={overlayTtlMs}
              onChange={setOverlayTtlMs}
              onCommit={() => void saveConfig()}
              unit="ms"
              min={0}
              max={30000}
              aria-label={t('autocomplete.overlayTtlMs')}
              data-testid="autocomplete-overlay-ttl-ms"
            />
          }
        />

        <div className="flex items-center gap-2 px-4 py-3">
          <Button
            type="button"
            variant="primary"
            size="sm"
            onClick={() => void saveConfig()}
            disabled={isSaving || !configLoaded}>
            {isSaving ? t('autocomplete.saving') : t('autocomplete.saveSettings')}
          </Button>
        </div>
      </SettingsSection>

      {/* ── Runtime ────────────────────────────────────────────────── */}
      <SettingsSection title={t('autocomplete.runtime')}>
        <div className="px-4 py-3 text-sm text-content-secondary space-y-1">
          <div>
            {t('autocomplete.running')}: {status?.running ? t('common.yes') : t('common.no')}
          </div>
          <div>
            {t('common.enabled')}: {status?.enabled ? t('common.yes') : t('common.no')}
          </div>
        </div>
        <div className="flex gap-2 px-4 pb-3">
          <Button
            type="button"
            variant="secondary"
            size="sm"
            onClick={() => void start()}
            disabled={!configLoaded || !status?.platform_supported || Boolean(status?.running)}>
            {t('autocomplete.start')}
          </Button>
          <Button
            type="button"
            variant="secondary"
            tone="danger"
            size="sm"
            onClick={() => void stop()}
            disabled={!status?.running}>
            {t('autocomplete.stop')}
          </Button>
        </div>
      </SettingsSection>

      {/* ── Status messages ─────────────────────────────────────────── */}
      <SettingsStatusLine
        saving={isSaving}
        savedNote={message}
        error={error}
        savingLabel={t('autocomplete.saving')}
      />

      {/* ── Advanced link ────────────────────────────────────────────── */}
      <Button
        type="button"
        variant="tertiary"
        size="xs"
        onClick={() => navigateToSettings('autocomplete-debug')}
        trailingIcon={
          <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
          </svg>
        }>
        {t('autocomplete.advancedSettings')}
      </Button>
    </SettingsPanel>
  );
};

export default AutocompletePanel;
