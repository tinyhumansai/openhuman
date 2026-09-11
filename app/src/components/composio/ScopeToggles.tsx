import type { ComposioUserScopePref } from '../../lib/composio/types';
import { useT } from '../../lib/i18n/I18nContext';
import { Switch } from '../ui';

type ScopeRowDef = { key: keyof ComposioUserScopePref; labelKey: string; hintKey: string };

const SCOPE_ROWS: Array<ScopeRowDef> = [
  {
    key: 'read',
    labelKey: 'composio.connect.scope.read',
    hintKey: 'composio.connect.scope.readHint',
  },
  {
    key: 'write',
    labelKey: 'composio.connect.scope.write',
    hintKey: 'composio.connect.scope.writeHint',
  },
  {
    key: 'admin',
    labelKey: 'composio.connect.scope.admin',
    hintKey: 'composio.connect.scope.adminHint',
  },
];

interface ScopeTogglesProps {
  scopes: ComposioUserScopePref | null;
  savingScope: keyof ComposioUserScopePref | null;
  onToggle: (key: keyof ComposioUserScopePref) => void;
  error: string | null;
}

export function ScopeToggles({ scopes, savingScope, onToggle, error }: ScopeTogglesProps) {
  const { t } = useT();
  // Render skeleton placeholders while we wait on the initial load so
  // the modal layout doesn't jump when the pref arrives.
  const loading = scopes === null;

  return (
    <div className="border-t border-line-subtle pt-3 mt-1 space-y-2">
      <div className="flex items-baseline justify-between">
        <h3 className="text-xs font-semibold text-content-secondary uppercase tracking-wide">
          {t('composio.connect.permissions')}
        </h3>
        <p className="text-[10px] text-content-faint">{t('composio.connect.permissionsDefault')}</p>
      </div>
      <ul className="space-y-1.5">
        {SCOPE_ROWS.map(row => {
          const enabled = scopes?.[row.key] ?? false;
          const isSaving = savingScope === row.key;
          const rowLabel = t(row.labelKey as Parameters<typeof t>[0]);
          const rowHint = t(row.hintKey as Parameters<typeof t>[0]);
          return (
            <li
              key={row.key}
              className="flex items-start justify-between gap-3 rounded-lg px-2 py-1.5 hover:bg-surface-hover">
              <div className="min-w-0 flex-1">
                <span className="text-sm font-medium text-content">{rowLabel}</span>
                <p className="text-[11px] text-content-faint leading-snug">{rowHint}</p>
              </div>
              <Switch
                id={`composio-scope-${row.key}`}
                checked={enabled}
                onCheckedChange={() => onToggle(row.key)}
                aria-label={`${enabled ? t('common.disable') : t('common.enable')} ${rowLabel} scope`}
                disabled={loading || savingScope !== null}
                className={isSaving ? 'animate-pulse' : undefined}
              />
            </li>
          );
        })}
      </ul>
      {error && <p className="text-[11px] text-coral-600">{error}</p>}
    </div>
  );
}
