import { RefreshCw, RotateCcw } from 'lucide-react';
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
import { formatCurrency } from '../../dashboard/formatCurrency';
import { Button, Card, Field, NumberField, StatusLine, Switch } from '../../ui';
import SettingsPanel from '../layout/SettingsPanel';

function formatInt(n: number): string {
  return Math.round(n).toLocaleString();
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
  <div className="rounded-lg bg-surface-muted px-3 py-2.5">
    <div className="text-xs text-content-muted">{label}</div>
    <div className="mt-0.5 text-xl font-semibold tabular-nums text-content">{value}</div>
    {hint && <div className="mt-0.5 truncate text-xs text-content-faint">{hint}</div>}
  </div>
);

interface ToggleRowProps {
  id: string;
  label: string;
  description: string;
  checked: boolean;
  disabled: boolean;
  onChange: (next: boolean) => void;
}

/** One labelled switch row; the label is wired to the switch through `htmlFor`. */
const ToggleRow = ({ id, label, description, checked, disabled, onChange }: ToggleRowProps) => (
  <Field
    htmlFor={id}
    label={label}
    description={description}
    control={
      <Switch
        id={id}
        checked={checked}
        disabled={disabled}
        onCheckedChange={onChange}
        aria-label={label}
      />
    }
  />
);

interface TokenUsagePanelProps {
  /** When true, render without the SettingsPanel chrome (used when embedded as
   *  a tab inside the Usage & limits surface). */
  embedded?: boolean;
}

const TokenUsagePanel = ({ embedded = false }: TokenUsagePanelProps = {}) => {
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
        const s = await getTokenjuiceSettings();
        if (cancelled) return;
        setSettings(s);
        setMinTokensInput(String(s.ccr_min_tokens));
        savedMinTokensRef.current = s.ccr_min_tokens;
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
        // Settings failure disables all controls — don't bother loading savings.
        return;
      }
      // Load savings independently: a savings failure must not prevent the
      // configuration controls from becoming interactive.
      try {
        const v = await getTokenjuiceSavings();
        if (!cancelled) setSavings(v);
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

  const body = (
    <div className="space-y-4">
      {/* ── Savings statistics ─────────────────────────────────────────── */}
      <Card
        title={t('settings.tokenUsage.savingsTitle')}
        description={
          savings
            ? t('settings.tokenUsage.attributedTo').replace('{model}', savings.attributionModel)
            : undefined
        }
        headerRight={
          <div className="flex gap-2">
            <Button
              variant="secondary"
              size="sm"
              leadingIcon={<RefreshCw className="h-3.5 w-3.5" aria-hidden />}
              onClick={() => void loadSavings()}>
              {t('settings.tokenUsage.refresh')}
            </Button>
            <Button
              variant="secondary"
              size="sm"
              leadingIcon={<RotateCcw className="h-3.5 w-3.5" aria-hidden />}
              onClick={() => void onReset()}>
              {t('settings.tokenUsage.reset')}
            </Button>
          </div>
        }>
        <div className="grid grid-cols-2 gap-2 p-4 lg:grid-cols-4">
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
            value={total ? formatCurrency(total.costSavedUsd, 'USD') : '—'}
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
          <div className="px-4 py-3">
            <div className="mb-1 text-xs font-medium text-content-muted">
              {t('settings.tokenUsage.byCompressor')}
            </div>
            <div className="divide-y divide-line-subtle">
              {Object.entries(savings.byCompressor)
                .sort((a, b) => b[1].tokensSaved - a[1].tokensSaved)
                .map(([name, b]) => (
                  <div key={name} className="flex items-center justify-between py-1.5 text-sm">
                    <span className="font-mono text-content-secondary">{name}</span>
                    <span className="tabular-nums text-content-muted">
                      {t('settings.tokenUsage.tokensAndCost', '{tokens} tok · {cost}')
                        .replace('{tokens}', formatInt(b.tokensSaved))
                        .replace('{cost}', formatCurrency(b.costSavedUsd, 'USD'))}
                    </span>
                  </div>
                ))}
            </div>
          </div>
        )}
      </Card>

      {/* ── Compression toggles ────────────────────────────────────────── */}
      <Card
        title={t('settings.tokenUsage.compressionTitle')}
        description={t('settings.tokenUsage.compressionDesc')}>
        <ToggleRow
          id="tj-router-enabled"
          label={t('settings.tokenUsage.routerEnabled')}
          description={t('settings.tokenUsage.routerEnabledDesc')}
          checked={settings?.router_enabled ?? false}
          disabled={settings === null}
          onChange={v => void patch({ router_enabled: v })}
        />
        <ToggleRow
          id="tj-search-enabled"
          label={t('settings.tokenUsage.search')}
          description={t('settings.tokenUsage.searchDesc')}
          checked={settings?.search_enabled ?? false}
          disabled={settings === null}
          onChange={v => void patch({ search_enabled: v })}
        />
        <ToggleRow
          id="tj-code-enabled"
          label={t('settings.tokenUsage.code')}
          description={t('settings.tokenUsage.codeDesc')}
          checked={settings?.code_enabled ?? false}
          disabled={settings === null}
          onChange={v => void patch({ code_enabled: v })}
        />
        <ToggleRow
          id="tj-html-enabled"
          label={t('settings.tokenUsage.html')}
          description={t('settings.tokenUsage.htmlDesc')}
          checked={settings?.html_enabled ?? false}
          disabled={settings === null}
          onChange={v => void patch({ html_enabled: v })}
        />
      </Card>

      {/* ── CCR cache ──────────────────────────────────────────────────── */}
      <Card
        title={t('settings.tokenUsage.ccrTitle')}
        description={t('settings.tokenUsage.ccrDesc')}>
        <ToggleRow
          id="tj-ccr-enabled"
          label={t('settings.tokenUsage.ccrEnabled')}
          description={t('settings.tokenUsage.ccrEnabledDesc')}
          checked={settings?.ccr_enabled ?? false}
          disabled={settings === null}
          onChange={v => void patch({ ccr_enabled: v })}
        />
        <Field
          htmlFor="tj-ccr-min-tokens"
          label={t('settings.tokenUsage.ccrMinTokens')}
          description={t('settings.tokenUsage.ccrMinTokensDesc')}
          control={
            <NumberField
              id="tj-ccr-min-tokens"
              value={minTokensInput}
              onChange={setMinTokensInput}
              onCommit={commitMinTokens}
              min={0}
              max={1000000}
              unit={t('settings.tokenUsage.tokensUnit')}
              disabled={settings === null}
              aria-label={t('settings.tokenUsage.ccrMinTokens')}
            />
          }
        />
        <ToggleRow
          id="tj-ccr-disk"
          label={t('settings.tokenUsage.ccrDisk')}
          description={t('settings.tokenUsage.ccrDiskDesc')}
          checked={settings?.ccr_disk_enabled ?? false}
          disabled={settings === null}
          onChange={v => void patch({ ccr_disk_enabled: v })}
        />
      </Card>

      <StatusLine
        saving={saving}
        savedNote={savedNote}
        error={error}
        savingLabel={t('settings.tokenUsage.saving')}
      />
    </div>
  );

  if (embedded) return body;
  return <SettingsPanel description={t('settings.tokenUsage.menuDesc')}>{body}</SettingsPanel>;
};

export default TokenUsagePanel;
