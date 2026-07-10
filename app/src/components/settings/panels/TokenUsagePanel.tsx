import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  getTokenjuiceSavings,
  getTokenjuiceSettings,
  resetTokenjuiceSavings,
  type SavingsStats,
  type TokenjuiceSettings,
  type TokenjuiceSettingsPatch,
  updateTokenjuiceSettings,
} from '../../../utils/tauriCommands/tokenjuice';
import Button from '../../ui/Button';
import {
  SettingsNumberField,
  SettingsRow,
  SettingsSection,
  SettingsStatusLine,
  SettingsSwitch,
} from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

function formatInt(n: number): string {
  return Math.round(n).toLocaleString();
}

function formatUsd(n: number): string {
  if (n > 0 && n < 0.01) return '<$0.01';
  return `$${n.toFixed(2)}`;
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

interface StatTileProps {
  label: string;
  value: string;
  hint?: string;
}

const StatTile = ({ label, value, hint }: StatTileProps) => (
  <div className="rounded-2xl border border-stone-200 dark:border-neutral-800 p-4 bg-gradient-to-br from-white to-stone-50 dark:from-neutral-900 dark:to-neutral-950">
    <div className="text-xs font-medium text-neutral-500 dark:text-neutral-400">{label}</div>
    <div className="mt-1 text-2xl font-semibold text-neutral-900 dark:text-neutral-50 tabular-nums">
      {value}
    </div>
    {hint && <div className="mt-0.5 text-xs text-neutral-400 dark:text-neutral-500">{hint}</div>}
  </div>
);

const TokenUsagePanel = () => {
  const { t } = useT();

  const [settings, setSettings] = useState<TokenjuiceSettings | null>(null);
  const [savings, setSavings] = useState<SavingsStats | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [savedNote, setSavedNote] = useState<string | null>(null);

  // Local editable string for the token-threshold field.
  const [minTokensInput, setMinTokensInput] = useState('');
  const savedMinTokensRef = useRef<number | null>(null);

  const loadSavings = useCallback(async () => {
    try {
      setSavings(await getTokenjuiceSavings());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const [s, v] = await Promise.all([getTokenjuiceSettings(), getTokenjuiceSavings()]);
        if (cancelled) return;
        setSettings(s);
        setSavings(v);
        setMinTokensInput(String(s.ccr_min_tokens));
        savedMinTokensRef.current = s.ccr_min_tokens;
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, []);

  const patch = useCallback(
    async (p: TokenjuiceSettingsPatch) => {
      setSaving(true);
      setError(null);
      setSavedNote(null);
      try {
        const next = await updateTokenjuiceSettings(p);
        setSettings(next);
        setMinTokensInput(String(next.ccr_min_tokens));
        savedMinTokensRef.current = next.ccr_min_tokens;
        setSavedNote(t('settings.tokenUsage.saved'));
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setSaving(false);
      }
    },
    [t]
  );

  const commitMinTokens = useCallback(() => {
    const parsed = Number.parseInt(minTokensInput, 10);
    if (!Number.isFinite(parsed) || parsed < 0) {
      setMinTokensInput(String(savedMinTokensRef.current ?? 0));
      return;
    }
    if (parsed === savedMinTokensRef.current) return;
    void patch({ ccr_min_tokens: parsed });
  }, [minTokensInput, patch]);

  const onReset = useCallback(async () => {
    try {
      await resetTokenjuiceSavings();
      await loadSavings();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [loadSavings]);

  const total = savings?.total;

  return (
    <SettingsPanel description={t('settings.tokenUsage.menuDesc')}>
      {/* ── Savings statistics ─────────────────────────────────────────── */}
      <SettingsSection
        title={t('settings.tokenUsage.savingsTitle')}
        description={
          savings
            ? t('settings.tokenUsage.attributedTo').replace('{model}', savings.attributionModel)
            : undefined
        }>
        <div className="grid grid-cols-2 md:grid-cols-4 gap-3 px-1">
          <StatTile
            label={t('settings.tokenUsage.tokensSaved')}
            value={total ? formatInt(total.tokensSaved) : '—'}
            hint={
              total
                ? t('settings.tokenUsage.overEvents').replace('{count}', formatInt(total.events))
                : undefined
            }
          />
          <StatTile
            label={t('settings.tokenUsage.costSaved')}
            value={total ? formatUsd(total.costSavedUsd) : '—'}
          />
          <StatTile
            label={t('settings.tokenUsage.cacheOccupancy')}
            value={savings ? formatInt(savings.cache.entries) : '—'}
            hint={savings ? formatBytes(savings.cache.bytes) : undefined}
          />
          <StatTile
            label={t('settings.tokenUsage.compactions')}
            value={total ? formatInt(total.events) : '—'}
          />
        </div>

        {savings && Object.keys(savings.byCompressor).length > 0 && (
          <div className="px-1 mt-3">
            <div className="text-xs font-medium text-neutral-500 dark:text-neutral-400 mb-1.5">
              {t('settings.tokenUsage.byCompressor')}
            </div>
            <div className="rounded-xl border border-stone-200 dark:border-neutral-800 divide-y divide-stone-100 dark:divide-neutral-800">
              {Object.entries(savings.byCompressor)
                .sort((a, b) => b[1].tokensSaved - a[1].tokensSaved)
                .map(([name, b]) => (
                  <div key={name} className="flex items-center justify-between px-3 py-2 text-sm">
                    <span className="font-mono text-neutral-700 dark:text-neutral-300">{name}</span>
                    <span className="tabular-nums text-neutral-500 dark:text-neutral-400">
                      {formatInt(b.tokensSaved)} tok · {formatUsd(b.costSavedUsd)}
                    </span>
                  </div>
                ))}
            </div>
          </div>
        )}

        <div className="px-1 mt-3 flex items-center gap-2">
          <Button variant="secondary" size="sm" onClick={() => void loadSavings()}>
            {t('settings.tokenUsage.refresh')}
          </Button>
          <Button variant="secondary" size="sm" onClick={() => void onReset()}>
            {t('settings.tokenUsage.reset')}
          </Button>
        </div>
      </SettingsSection>

      {/* ── Compression toggles ────────────────────────────────────────── */}
      <SettingsSection
        title={t('settings.tokenUsage.compressionTitle')}
        description={t('settings.tokenUsage.compressionDesc')}>
        <div className="rounded-xl border border-stone-200 dark:border-neutral-800 divide-y divide-stone-100 dark:divide-neutral-800">
          <SettingsRow
            label={t('settings.tokenUsage.routerEnabled')}
            description={t('settings.tokenUsage.routerEnabledDesc')}
            control={
              <SettingsSwitch
                id="tj-router-enabled"
                checked={settings?.router_enabled ?? false}
                onCheckedChange={v => void patch({ router_enabled: v })}
                aria-label={t('settings.tokenUsage.routerEnabled')}
              />
            }
          />
          <SettingsRow
            label={t('settings.tokenUsage.search')}
            description={t('settings.tokenUsage.searchDesc')}
            control={
              <SettingsSwitch
                id="tj-search-enabled"
                checked={settings?.search_enabled ?? false}
                onCheckedChange={v => void patch({ search_enabled: v })}
                aria-label={t('settings.tokenUsage.search')}
              />
            }
          />
          <SettingsRow
            label={t('settings.tokenUsage.code')}
            description={t('settings.tokenUsage.codeDesc')}
            control={
              <SettingsSwitch
                id="tj-code-enabled"
                checked={settings?.code_enabled ?? false}
                onCheckedChange={v => void patch({ code_enabled: v })}
                aria-label={t('settings.tokenUsage.code')}
              />
            }
          />
          <SettingsRow
            label={t('settings.tokenUsage.html')}
            description={t('settings.tokenUsage.htmlDesc')}
            control={
              <SettingsSwitch
                id="tj-html-enabled"
                checked={settings?.html_enabled ?? false}
                onCheckedChange={v => void patch({ html_enabled: v })}
                aria-label={t('settings.tokenUsage.html')}
              />
            }
          />
          <SettingsRow
            label={t('settings.tokenUsage.ml')}
            description={t('settings.tokenUsage.mlDesc')}
            control={
              <SettingsSwitch
                id="tj-ml-enabled"
                checked={settings?.ml_compression_enabled ?? false}
                onCheckedChange={v => void patch({ ml_compression_enabled: v })}
                aria-label={t('settings.tokenUsage.ml')}
              />
            }
          />
        </div>
      </SettingsSection>

      {/* ── CCR cache ──────────────────────────────────────────────────── */}
      <SettingsSection
        title={t('settings.tokenUsage.ccrTitle')}
        description={t('settings.tokenUsage.ccrDesc')}>
        <div className="rounded-xl border border-stone-200 dark:border-neutral-800 divide-y divide-stone-100 dark:divide-neutral-800">
          <SettingsRow
            label={t('settings.tokenUsage.ccrEnabled')}
            description={t('settings.tokenUsage.ccrEnabledDesc')}
            control={
              <SettingsSwitch
                id="tj-ccr-enabled"
                checked={settings?.ccr_enabled ?? false}
                onCheckedChange={v => void patch({ ccr_enabled: v })}
                aria-label={t('settings.tokenUsage.ccrEnabled')}
              />
            }
          />
          <SettingsRow
            stacked
            label={t('settings.tokenUsage.ccrMinTokens')}
            description={t('settings.tokenUsage.ccrMinTokensDesc')}
            control={
              <SettingsNumberField
                id="tj-ccr-min-tokens"
                value={minTokensInput}
                onChange={setMinTokensInput}
                onCommit={commitMinTokens}
                min={0}
                max={1000000}
                unit={t('settings.tokenUsage.tokensUnit')}
                aria-label={t('settings.tokenUsage.ccrMinTokens')}
              />
            }
          />
          <SettingsRow
            label={t('settings.tokenUsage.ccrDisk')}
            description={t('settings.tokenUsage.ccrDiskDesc')}
            control={
              <SettingsSwitch
                id="tj-ccr-disk"
                checked={settings?.ccr_disk_enabled ?? false}
                onCheckedChange={v => void patch({ ccr_disk_enabled: v })}
                aria-label={t('settings.tokenUsage.ccrDisk')}
              />
            }
          />
        </div>
        <div className="px-1 mt-2">
          <SettingsStatusLine
            saving={saving}
            savedNote={savedNote}
            error={error}
            savingLabel={t('settings.tokenUsage.saving')}
          />
        </div>
      </SettingsSection>
    </SettingsPanel>
  );
};

export default TokenUsagePanel;
