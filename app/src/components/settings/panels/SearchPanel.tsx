import { useEffect, useId, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { useCoreState } from '../../../providers/CoreStateProvider';
import { isLocalSessionToken } from '../../../utils/localSession';
import {
  openhumanGetSearchSettings,
  openhumanUpdateSearchSettings,
  type SearchEngineId,
  type SearchSettings,
  type SearchSettingsUpdate,
} from '../../../utils/tauriCommands/config';
import PanelPage from '../../layout/PanelPage';
import Button from '../../ui/Button';
import Input from '../../ui/Input';
import SettingsBackButton from '../components/SettingsBackButton';
import { SettingsStatusLine, SettingsTextArea } from '../controls';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

type Status =
  | { kind: 'idle' }
  | { kind: 'loading' }
  | { kind: 'saving' }
  | { kind: 'saved' }
  | { kind: 'error'; message: string };

/**
 * Tri-state web-access mode for the unified fetch + browser allowlist.
 * - `all`    → `allow_all: true` (the `"*"` wildcard)
 * - `custom` → `allow_all: false` + an explicit host list (textarea)
 * - `block`  → `allow_all: false` + an empty host list (no web access)
 *
 * `block` and an empty `custom` are indistinguishable once persisted (both are
 * `allow_all: false` + `[]`); the distinction only matters locally while
 * editing.
 */
type AccessMode = 'all' | 'custom' | 'block';

interface EngineOption {
  id: SearchEngineId;
  label: string;
  description: string;
  requiresKey: boolean;
}

/**
 * Normalize a user-entered allowed-site entry down to a bare host so it
 * matches `url_guard`'s host-based comparison. Strips a leading scheme and any
 * path/query/fragment — e.g. `https://reuters.com/markets` → `reuters.com` —
 * and trims surrounding whitespace. The `*` allow-all wildcard is preserved.
 */
const normalizeAllowedHost = (raw: string): string =>
  raw
    .trim()
    .replace(/^[a-z][a-z0-9+.-]*:\/\//i, '')
    .replace(/\/.*$/, '')
    .trim();

const SearchPanel = ({ embedded = false }: { embedded?: boolean }) => {
  const { t } = useT();
  const { navigateBack } = useSettingsNavigation();
  const { snapshot } = useCoreState();
  const isLocalSession = isLocalSessionToken(snapshot.sessionToken);

  const [settings, setSettings] = useState<SearchSettings | null>(null);
  const [status, setStatus] = useState<Status>({ kind: 'loading' });
  const [parallelKey, setParallelKey] = useState<string>('');
  const [braveKey, setBraveKey] = useState<string>('');
  const [queritKey, setQueritKey] = useState<string>('');
  const [showParallel, setShowParallel] = useState(false);
  const [showBrave, setShowBrave] = useState(false);
  const [showQuerit, setShowQuerit] = useState(false);
  // Editor text for the allowed-websites host list (one host per line). The
  // "*" wildcard is represented by the access mode, not shown here.
  const [allowedText, setAllowedText] = useState<string>('');
  // Tri-state web-access mode for the unified fetch + browser allowlist.
  const [mode, setMode] = useState<AccessMode>('all');
  // Sync editor + mode from settings exactly once, so a later settings refresh
  // (e.g. after saving an engine change) can't clobber the user's in-progress
  // host edits or chosen mode.
  const initializedRef = useRef(false);

  const ENGINES: EngineOption[] = [
    {
      id: 'disabled',
      label: t('settings.search.engineDisabledLabel'),
      description: t('settings.search.engineDisabledDesc'),
      requiresKey: false,
    },
    {
      id: 'managed',
      label: t('settings.search.engineManagedLabel'),
      description: t('settings.search.engineManagedDesc'),
      requiresKey: false,
    },
    {
      id: 'parallel',
      label: t('settings.search.engineParallelLabel'),
      description: t('settings.search.engineParallelDesc'),
      requiresKey: true,
    },
    {
      id: 'brave',
      label: t('settings.search.engineBraveLabel'),
      description: t('settings.search.engineBraveDesc'),
      requiresKey: true,
    },
    {
      id: 'querit',
      label: t('settings.search.engineQueritLabel'),
      description: t('settings.search.engineQueritDesc'),
      requiresKey: true,
    },
  ];
  const visibleEngines = isLocalSession
    ? ENGINES.filter(engine => engine.id !== 'managed')
    : ENGINES;

  useEffect(() => {
    let cancelled = false;
    openhumanGetSearchSettings()
      .then(res => {
        if (cancelled) return;
        setSettings(res.result);
        setStatus({ kind: 'idle' });
      })
      .catch(err => {
        if (cancelled) return;
        setStatus({ kind: 'error', message: err instanceof Error ? err.message : String(err) });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // Reflect the loaded allowlist into the editor + mode, exactly once.
  useEffect(() => {
    if (!settings || initializedRef.current) return;
    initializedRef.current = true;
    const explicit = settings.allowed_domains.filter(d => d !== '*');
    setAllowedText(explicit.join('\n'));
    setMode(settings.allow_all ? 'all' : explicit.length > 0 ? 'custom' : 'block');
  }, [settings]);

  const selectedEngine = (settings?.engine as SearchEngineId | undefined) ?? 'managed';

  const persistEngine = async (next: SearchEngineId) => {
    if (!settings || status.kind === 'saving') return;
    const previous = settings;
    setSettings({ ...settings, engine: next });
    setStatus({ kind: 'saving' });
    try {
      await openhumanUpdateSearchSettings({ engine: next });
      const refreshed = await openhumanGetSearchSettings();
      setSettings(refreshed.result);
      setStatus({ kind: 'saved' });
    } catch (err) {
      setSettings(previous);
      setStatus({ kind: 'error', message: err instanceof Error ? err.message : String(err) });
    }
  };

  const persistKey = async (engine: 'parallel' | 'brave' | 'querit', rawKey: string) => {
    if (!settings) return;
    setStatus({ kind: 'saving' });
    try {
      const update =
        engine === 'parallel'
          ? { parallel_api_key: rawKey }
          : engine === 'brave'
            ? { brave_api_key: rawKey }
            : { querit_api_key: rawKey };
      await openhumanUpdateSearchSettings(update);
      const refreshed = await openhumanGetSearchSettings();
      setSettings(refreshed.result);
      if (engine === 'parallel') setParallelKey('');
      else if (engine === 'brave') setBraveKey('');
      else setQueritKey('');
      setStatus({ kind: 'saved' });
    } catch (err) {
      setStatus({ kind: 'error', message: err instanceof Error ? err.message : String(err) });
    }
  };

  const persistSearchUpdate = async (update: SearchSettingsUpdate) => {
    if (!settings || status.kind === 'saving') return;
    setStatus({ kind: 'saving' });
    try {
      await openhumanUpdateSearchSettings(update);
      const refreshed = await openhumanGetSearchSettings();
      setSettings(refreshed.result);
      setStatus({ kind: 'saved' });
    } catch (err) {
      setStatus({ kind: 'error', message: err instanceof Error ? err.message : String(err) });
    }
  };

  // Switch web-access mode. "Allow all" / "Block all" persist immediately;
  // "Custom" only reveals the host editor (its Save button persists the list),
  // and we keep whatever the user has already typed.
  const selectMode = (next: AccessMode) => {
    if (status.kind === 'saving') return;
    setMode(next);
    if (next === 'all') {
      void persistSearchUpdate({ allow_all: true });
    } else if (next === 'block') {
      void persistSearchUpdate({ allowed_domains: [], allow_all: false });
    }
  };

  const persistAllowedDomains = () => {
    const domains = allowedText.split('\n').map(normalizeAllowedHost).filter(Boolean);
    // Editing the explicit host list implies "not allow-all".
    void persistSearchUpdate({ allowed_domains: domains, allow_all: false });
  };

  const isConfigured = (engine: SearchEngineId): boolean => {
    if (!settings) return false;
    if (engine === 'disabled') return true;
    if (engine === 'managed') return true;
    if (engine === 'parallel') return settings.parallel_configured;
    if (engine === 'brave') return settings.brave_configured;
    if (engine === 'querit') return settings.querit_configured;
    return false;
  };

  return (
    <PanelPage
      className="z-10"
      testId="search-settings-panel"
      contentClassName=""
      description={embedded ? undefined : t('settings.search.menuDesc')}
      leading={embedded ? undefined : <SettingsBackButton onBack={navigateBack} />}>
      <div className={embedded ? 'space-y-5' : 'p-4 space-y-5'}>
        <p className="text-xs text-content-muted leading-relaxed">
          {t('settings.search.description')}
        </p>

        {isLocalSession && (
          <div className="rounded-lg border border-line bg-surface-muted px-4 py-3 text-sm text-content-secondary">
            {t('settings.search.localManagedUnavailable')}
          </div>
        )}

        {status.kind === 'loading' && (
          <div className="rounded-lg border border-line bg-surface p-4 text-xs text-content-muted">
            {t('common.loading')}
          </div>
        )}

        {settings && (
          <>
            <div
              className="bg-surface rounded-xl border border-line overflow-hidden"
              role="radiogroup"
              aria-label={t('settings.search.engineAria')}>
              {visibleEngines.map((opt, idx) => {
                const selected = opt.id === selectedEngine;
                const configured = isConfigured(opt.id);
                const blocked = opt.requiresKey && !configured && selected;
                return (
                  <button
                    key={opt.id}
                    type="button"
                    data-testid={`search-engine-${opt.id}`}
                    role="radio"
                    aria-checked={selected}
                    onClick={() => void persistEngine(opt.id)}
                    className={`w-full flex items-start gap-3 px-4 py-3 text-left transition-colors focus:outline-none focus-visible:bg-primary-50 dark:focus-visible:bg-primary-900/30 ${
                      idx !== 0 ? 'border-t border-line-subtle' : ''
                    } ${
                      selected ? 'bg-primary-50 dark:bg-primary-500/10' : 'hover:bg-surface-hover'
                    }`}>
                    <span className="flex-1 min-w-0">
                      <span className="flex items-center gap-2">
                        <span className="text-sm font-medium text-content">{opt.label}</span>
                        {opt.requiresKey && (
                          <span
                            className={`inline-flex items-center px-1.5 py-0.5 rounded text-[9px] font-semibold uppercase tracking-wider ${
                              configured
                                ? 'bg-sage-100 text-sage-700 dark:bg-sage-900/40 dark:text-sage-200'
                                : 'bg-amber-100 text-amber-800 dark:bg-amber-900/40 dark:text-amber-200'
                            }`}>
                            {configured
                              ? t('settings.search.statusConfigured')
                              : t('settings.search.statusNeedsKey')}
                          </span>
                        )}
                      </span>
                      <span className="block mt-0.5 text-xs text-content-muted">
                        {opt.description}
                      </span>
                      {blocked && (
                        <span className="block mt-1 text-[11px] text-amber-700 dark:text-amber-300">
                          {t('settings.search.fallbackToManaged')}
                        </span>
                      )}
                    </span>
                    {selected && (
                      <svg
                        className="w-5 h-5 text-primary-500 flex-shrink-0 mt-0.5"
                        fill="none"
                        stroke="currentColor"
                        viewBox="0 0 24 24"
                        aria-hidden>
                        <path
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          strokeWidth={2}
                          d="M5 13l4 4L19 7"
                        />
                      </svg>
                    )}
                  </button>
                );
              })}
            </div>

            {/* BYO API keys */}
            <div className="space-y-3">
              <KeyEditor
                label={t('settings.search.parallelKeyLabel')}
                placeholder={
                  settings.parallel_configured
                    ? t('settings.search.placeholderStored')
                    : t('settings.search.placeholderParallel')
                }
                show={showParallel}
                onToggleShow={() => setShowParallel(s => !s)}
                value={parallelKey}
                onChange={setParallelKey}
                onSave={() => void persistKey('parallel', parallelKey)}
                onClear={() => void persistKey('parallel', '')}
                configured={settings.parallel_configured}
                docUrl="https://parallel.ai/"
                t={t}
              />
              <KeyEditor
                label={t('settings.search.braveKeyLabel')}
                placeholder={
                  settings.brave_configured
                    ? t('settings.search.placeholderStored')
                    : t('settings.search.placeholderBrave')
                }
                show={showBrave}
                onToggleShow={() => setShowBrave(s => !s)}
                value={braveKey}
                onChange={setBraveKey}
                onSave={() => void persistKey('brave', braveKey)}
                onClear={() => void persistKey('brave', '')}
                configured={settings.brave_configured}
                docUrl="https://brave.com/search/api/"
                t={t}
              />
              <KeyEditor
                label={t('settings.search.queritKeyLabel')}
                placeholder={
                  settings.querit_configured
                    ? t('settings.search.placeholderStored')
                    : t('settings.search.placeholderQuerit')
                }
                show={showQuerit}
                onToggleShow={() => setShowQuerit(s => !s)}
                value={queritKey}
                onChange={setQueritKey}
                onSave={() => void persistKey('querit', queritKey)}
                onClear={() => void persistKey('querit', '')}
                configured={settings.querit_configured}
                docUrl="https://www.querit.ai/en/docs/reference/post"
                t={t}
              />
            </div>

            {/* Allowed websites — unified host allowlist shared by web_fetch /
                curl and (when enabled) the browser tool. Web search is not
                gated by this list. */}
            <div className="rounded-xl border border-line bg-surface p-3 space-y-2">
              {/* Section heading, not a form label — use a <p> so screen
                  readers don't announce an orphan <label> with no htmlFor. */}
              <p className="text-xs font-semibold text-content-secondary">
                {t('settings.search.allowedSitesLabel')}
              </p>
              <div
                role="radiogroup"
                aria-label={t('settings.search.accessModeAria')}
                className="flex rounded-lg border border-line overflow-hidden">
                {(
                  [
                    ['all', 'settings.search.accessAllowAll'],
                    ['custom', 'settings.search.accessCustom'],
                    ['block', 'settings.search.accessBlockAll'],
                  ] as const
                ).map(([value, labelKey], idx) => {
                  const selected = mode === value;
                  return (
                    <button
                      key={value}
                      type="button"
                      role="radio"
                      aria-checked={selected}
                      onClick={() => selectMode(value)}
                      disabled={status.kind === 'saving'}
                      className={`flex-1 px-3 py-1.5 text-xs font-medium transition-colors disabled:opacity-50 focus:outline-none focus-visible:bg-primary-50 dark:focus-visible:bg-primary-900/30 ${
                        idx !== 0 ? 'border-l border-line' : ''
                      } ${
                        selected
                          ? 'bg-primary-500 text-content-inverted'
                          : 'bg-surface text-content-secondary hover:bg-surface-hover'
                      }`}>
                      {t(labelKey)}
                    </button>
                  );
                })}
              </div>
              <p className="text-[11px] text-content-muted leading-relaxed">
                {mode === 'all'
                  ? t('settings.search.allowedSitesAllOn')
                  : mode === 'block'
                    ? t('settings.search.accessBlockAllHint')
                    : t('settings.search.allowedSitesHint')}
              </p>
              {mode === 'custom' && (
                <>
                  <SettingsTextArea
                    value={allowedText}
                    onChange={e => setAllowedText(e.target.value)}
                    rows={4}
                    spellCheck={false}
                    placeholder={t('settings.search.allowedSitesPlaceholder')}
                    className="font-mono text-xs"
                    aria-label={t('settings.search.allowedSitesLabel')}
                  />
                  <Button
                    type="button"
                    variant="primary"
                    size="xs"
                    onClick={() => persistAllowedDomains()}
                    disabled={status.kind === 'saving'}>
                    {t('settings.search.allowedSitesSave')}
                  </Button>
                </>
              )}
            </div>

            <SettingsStatusLine
              saving={status.kind === 'saving'}
              savedNote={status.kind === 'saved' ? t('settings.search.statusSaved') : null}
              error={
                status.kind === 'error'
                  ? `${t('settings.search.statusError')}: ${status.message}`
                  : null
              }
              savingLabel={t('settings.search.statusSaving')}
            />
          </>
        )}
      </div>
    </PanelPage>
  );
};

interface KeyEditorProps {
  label: string;
  placeholder: string;
  show: boolean;
  onToggleShow: () => void;
  value: string;
  onChange: (v: string) => void;
  onSave: () => void;
  onClear: () => void;
  configured: boolean;
  docUrl: string;
  t: (key: string) => string;
}

const KeyEditor = ({
  label,
  placeholder,
  show,
  onToggleShow,
  value,
  onChange,
  onSave,
  onClear,
  configured,
  docUrl,
  t,
}: KeyEditorProps) => {
  const inputId = useId();

  return (
    <div
      role="group"
      aria-labelledby={inputId}
      className="rounded-xl border border-line bg-surface p-3">
      <div className="flex items-center justify-between mb-2">
        <label
          id={inputId}
          htmlFor={`${inputId}-input`}
          className="text-xs font-semibold text-content">
          {label}
        </label>
        <a
          href={docUrl}
          target="_blank"
          rel="noopener noreferrer"
          className="text-[10px] text-primary-500 hover:underline">
          {t('settings.search.getApiKey')} ↗
        </a>
      </div>
      <div className="flex items-center gap-2">
        <Input
          id={`${inputId}-input`}
          type={show ? 'text' : 'password'}
          inputSize="sm"
          value={value}
          onChange={e => onChange(e.target.value)}
          placeholder={placeholder}
          className="flex-1 min-w-0 font-mono"
        />
        <Button type="button" variant="secondary" size="xs" onClick={onToggleShow}>
          {show ? t('settings.search.hide') : t('settings.search.show')}
        </Button>
        <Button
          type="button"
          variant="primary"
          size="xs"
          onClick={onSave}
          disabled={value.trim().length === 0}>
          {t('settings.search.save')}
        </Button>
        {configured && (
          <Button type="button" variant="secondary" tone="danger" size="xs" onClick={onClear}>
            {t('settings.search.clear')}
          </Button>
        )}
      </div>
    </div>
  );
};

export default SearchPanel;
