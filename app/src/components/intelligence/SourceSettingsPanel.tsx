/**
 * Expandable per-source sync-settings panel.
 *
 * Rendered inline below a SourceRow when the gear icon is clicked.
 * Shows only the limit fields relevant to the source's kind, seeded from
 * the current entry values. Empty input = omit the field on save (meaning
 * "use backend default / unlimited").
 *
 * On Save calls `updateMemorySource` and notifies the parent via
 * `onSaved` with the updated entry.
 */
import { useCallback, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  type MemorySourceEntry,
  type SourceKind,
  updateMemorySource,
} from '../../services/memorySourcesService';
import Button from '../ui/Button';
import Input from '../ui/Input';

// Which limit fields are relevant per kind. Order determines display order.
// Only caps that are actually enforced at sync time are surfaced — the
// per-sync token/cost budgets are not yet wired into the sync loop, so they
// are intentionally omitted here.
const KIND_FIELDS: Record<SourceKind, Array<keyof LimitFields>> = {
  composio: ['sync_depth_days', 'max_items'],
  conversation: ['sync_depth_days'],
  github_repo: ['max_prs', 'max_issues', 'max_commits', 'sync_depth_days'],
  rss_feed: ['max_items', 'sync_depth_days'],
  twitter_query: ['since_days'],
  web_page: ['sync_depth_days'],
  folder: ['sync_depth_days'],
};

// i18n key for each field label
const FIELD_LABEL_KEYS: Record<keyof LimitFields, string> = {
  max_prs: 'memorySources.settings.maxPrs',
  max_issues: 'memorySources.settings.maxIssues',
  max_commits: 'memorySources.settings.maxCommits',
  max_items: 'memorySources.settings.maxItems',
  since_days: 'memorySources.settings.sinceDays',
  sync_depth_days: 'memorySources.settings.syncDepthDays',
};

type LimitFields = Pick<
  MemorySourceEntry,
  'max_prs' | 'max_issues' | 'max_commits' | 'max_items' | 'since_days' | 'sync_depth_days'
>;

// Item-count caps where a "Maxed" badge is meaningful (synced count vs cap).
// Time-window (sync_depth_days/since_days) and budget (tokens/cost) caps don't
// map to a chunk count, so they never show "Maxed".
interface SourceSettingsPanelProps {
  source: MemorySourceEntry;
  onSaved: (updated: MemorySourceEntry) => void;
  onToast?: (toast: { type: 'success' | 'error'; title: string; message?: string }) => void;
}

export function SourceSettingsPanel({ source, onSaved, onToast }: SourceSettingsPanelProps) {
  const { t } = useT();
  const fields = KIND_FIELDS[source.kind] ?? [];

  // Hold each field as a string so inputs can be freely edited. Seeded from the
  // entry's stored cap (fetched from the backend — the single source of truth;
  // the caps migration writes conservative defaults onto the entry). An empty
  // field genuinely means "no cap / unlimited".
  const [values, setValues] = useState<Record<string, string>>(() => {
    const init: Record<string, string> = {};
    for (const f of fields) {
      const stored = source[f as keyof MemorySourceEntry];
      init[f] = stored != null ? String(stored) : '';
    }
    return init;
  });

  const [saving, setSaving] = useState(false);

  const handleChange = useCallback((field: string, value: string) => {
    setValues(prev => ({ ...prev, [field]: value }));
  }, []);

  const handleSave = useCallback(async () => {
    setSaving(true);
    try {
      const patch: Partial<LimitFields> = {};
      for (const f of fields) {
        const raw = values[f];
        if (raw !== '' && raw !== undefined) {
          const parsed = Number(raw);
          if (!Number.isFinite(parsed) || !Number.isInteger(parsed) || parsed < 0) {
            onToast?.({ type: 'error', title: t('memorySources.settings.saveFailed') });
            return;
          }
          (patch as Record<string, number>)[f] = parsed;
        }
        // Empty string → omit from patch (backend treats absence as "default")
      }
      const updated = await updateMemorySource(source.id, patch);
      onSaved(updated);
      onToast?.({ type: 'success', title: t('memorySources.settings.saved') });
    } catch (err) {
      onToast?.({
        type: 'error',
        title: t('memorySources.settings.saveFailed'),
        message: err instanceof Error ? err.message : String(err),
      });
    } finally {
      setSaving(false);
    }
  }, [fields, source.id, values, onSaved, onToast, t]);

  if (fields.length === 0) return null;

  // Display name for the tooltip — the toolkit slug (title-cased) for Composio
  // sources, else the source label.
  const toolkitName = source.toolkit
    ? source.toolkit.charAt(0).toUpperCase() + source.toolkit.slice(1)
    : source.label;
  const unlimitedTooltip = t('memorySources.settings.unlimitedTooltip').replace(
    '{toolkit}',
    toolkitName
  );

  return (
    <div
      className="mt-2 ml-7 rounded-lg border border-line bg-surface-muted p-3"
      data-testid={`source-settings-panel-${source.id}`}>
      <p className="mb-2 text-xs font-semibold text-content-secondary">
        {t('memorySources.settings.title')}
      </p>
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        {fields.map(field => {
          const isUnlimited = (values[field] ?? '') === '';
          // No "Maxed" badge. The only count a row has is *chunks*, and a cap
          // is *items* (emails, issues, commits); one email is several
          // chunks, so the badge lit long before the cap was reached. It
          // comes back when the status carries an item count (openhuman#6012).
          return (
            <div key={field}>
              <label
                htmlFor={`src-setting-${source.id}-${field}`}
                className="mb-0.5 flex items-center gap-1.5 text-xs font-medium text-content-secondary">
                {t(FIELD_LABEL_KEYS[field])}
                {isUnlimited && (
                  <span
                    className="inline-flex cursor-help text-content-faint"
                    title={unlimitedTooltip}
                    aria-label={unlimitedTooltip}>
                    <InfoIcon />
                  </span>
                )}
              </label>
              <Input
                id={`src-setting-${source.id}-${field}`}
                type="number"
                inputSize="sm"
                monospace
                min={0}
                step={1}
                value={values[field] ?? ''}
                onChange={e => handleChange(field, e.target.value)}
                placeholder={t('memorySources.settings.unlimited')}
                className="text-xs"
              />
            </div>
          );
        })}
      </div>
      <div className="mt-3 flex justify-end">
        <Button variant="primary" size="sm" onClick={() => void handleSave()} disabled={saving}>
          {saving ? t('memorySources.settings.saving') : t('memorySources.settings.save')}
        </Button>
      </div>
    </div>
  );
}

function InfoIcon() {
  return (
    <svg
      width="13"
      height="13"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true">
      <circle cx="12" cy="12" r="10" />
      <path d="M12 16v-4M12 8h.01" />
    </svg>
  );
}
