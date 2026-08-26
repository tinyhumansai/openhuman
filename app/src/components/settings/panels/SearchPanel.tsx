import { useEffect, useRef, useState } from 'react';

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
import { Alert, AlertDescription } from '../../ui/Alert';
import Button from '../../ui/Button';
import { CenteredLoadingState } from '../../ui/LoadingState';
import StatusLine from '../../ui/StatusLine';
import TextArea from '../../ui/TextArea';
import { ToggleGroupItem, ToggleGroupRoot } from '../../ui/ToggleGroup';
import SettingsBackButton from '../components/SettingsBackButton';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import SearchPanelEngineList, { type EngineOption } from './SearchPanelEngineList';
import KeyEditor from './SearchPanelKeyEditor';

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

/** Search engines that route directly from this machine with the user's own key. */
type ByokEngine = 'parallel' | 'brave' | 'querit' | 'exa' | 'tavily';

/** Patch field that carries each BYOK engine's key. Empty string clears it. */
const BYOK_KEY_FIELD: Record<ByokEngine, keyof SearchSettingsUpdate> = {
  parallel: 'parallel_api_key',
  brave: 'brave_api_key',
  querit: 'querit_api_key',
  exa: 'exa_api_key',
  tavily: 'tavily_api_key',
};

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
  const [exaKey, setExaKey] = useState<string>('');
  const [tavilyKey, setTavilyKey] = useState<string>('');
  const [showParallel, setShowParallel] = useState(false);
  const [showBrave, setShowBrave] = useState(false);
  const [showQuerit, setShowQuerit] = useState(false);
  const [showExa, setShowExa] = useState(false);
  const [showTavily, setShowTavily] = useState(false);
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
    {
      id: 'exa',
      label: t('settings.search.engineExaLabel'),
      description: t('settings.search.engineExaDesc'),
      requiresKey: true,
    },
    {
      id: 'tavily',
      label: t('settings.search.engineTavilyLabel'),
      description: t('settings.search.engineTavilyDesc'),
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

  // Clear the local draft input once its key round-trips to the core.
  const clearDraftKey: Record<ByokEngine, () => void> = {
    parallel: () => setParallelKey(''),
    brave: () => setBraveKey(''),
    querit: () => setQueritKey(''),
    exa: () => setExaKey(''),
    tavily: () => setTavilyKey(''),
  };

  const persistKey = async (engine: ByokEngine, rawKey: string) => {
    if (!settings) return;
    setStatus({ kind: 'saving' });
    try {
      await openhumanUpdateSearchSettings({ [BYOK_KEY_FIELD[engine]]: rawKey });
      const refreshed = await openhumanGetSearchSettings();
      setSettings(refreshed.result);
      clearDraftKey[engine]();
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
    if (engine === 'exa') return settings.exa_configured;
    if (engine === 'tavily') return settings.tavily_configured;
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
          <Alert variant="info">
            <AlertDescription>{t('settings.search.localManagedUnavailable')}</AlertDescription>
          </Alert>
        )}

        {status.kind === 'loading' && <CenteredLoadingState label={t('common.loading')} />}

        {settings && (
          <>
            <SearchPanelEngineList
              engines={visibleEngines}
              selectedEngine={selectedEngine}
              ariaLabel={t('settings.search.engineAria')}
              isConfigured={isConfigured}
              onSelect={engine => void persistEngine(engine)}
              t={t}
            />

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
              <KeyEditor
                label={t('settings.search.exaKeyLabel')}
                placeholder={
                  settings.exa_configured
                    ? t('settings.search.placeholderStored')
                    : t('settings.search.placeholderExa')
                }
                show={showExa}
                onToggleShow={() => setShowExa(s => !s)}
                value={exaKey}
                onChange={setExaKey}
                onSave={() => void persistKey('exa', exaKey)}
                onClear={() => void persistKey('exa', '')}
                configured={settings.exa_configured}
                docUrl="https://exa.ai"
                t={t}
              />
              <KeyEditor
                label={t('settings.search.tavilyKeyLabel')}
                placeholder={
                  settings.tavily_configured
                    ? t('settings.search.placeholderStored')
                    : t('settings.search.placeholderTavily')
                }
                show={showTavily}
                onToggleShow={() => setShowTavily(s => !s)}
                value={tavilyKey}
                onChange={setTavilyKey}
                onSave={() => void persistKey('tavily', tavilyKey)}
                onClear={() => void persistKey('tavily', '')}
                configured={settings.tavily_configured}
                docUrl="https://tavily.com"
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
              <ToggleGroupRoot
                type="single"
                aria-label={t('settings.search.accessModeAria')}
                value={mode}
                onValueChange={value => {
                  if (value) selectMode(value as AccessMode);
                }}
                disabled={status.kind === 'saving'}
                className="flex w-full rounded-lg border border-line overflow-hidden gap-0">
                {(
                  [
                    ['all', 'settings.search.accessAllowAll'],
                    ['custom', 'settings.search.accessCustom'],
                    ['block', 'settings.search.accessBlockAll'],
                  ] as const
                ).map(([value, labelKey]) => (
                  <ToggleGroupItem
                    key={value}
                    value={value}
                    variant="tertiary"
                    className="flex-1 rounded-none border-0 border-l border-line first:border-l-0 px-3 py-1.5 text-xs data-[state=on]:bg-primary-500 data-[state=on]:text-content-inverted">
                    {t(labelKey)}
                  </ToggleGroupItem>
                ))}
              </ToggleGroupRoot>
              <p className="text-[11px] text-content-muted leading-relaxed">
                {mode === 'all'
                  ? t('settings.search.allowedSitesAllOn')
                  : mode === 'block'
                    ? t('settings.search.accessBlockAllHint')
                    : t('settings.search.allowedSitesHint')}
              </p>
              {mode === 'custom' && (
                <>
                  <TextArea
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

            <StatusLine
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

export default SearchPanel;
