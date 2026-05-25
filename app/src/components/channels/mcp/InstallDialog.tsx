/**
 * Install dialog for a Smithery server.
 * Fetches the server detail, renders env-key inputs (password type with
 * show/hide toggle), an optional raw-JSON config textarea, and calls
 * `install` on submit.
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { mcpClientsApi } from '../../../services/api/mcpClientsApi';
import type { InstalledServer, SmitheryServerDetail } from './types';

const log = debug('mcp-clients:install');

interface InstallDialogProps {
  qualifiedName: string;
  prefillEnv?: Record<string, string>;
  onSuccess: (server: InstalledServer) => void;
  onCancel: () => void;
}

const InstallDialog = ({ qualifiedName, prefillEnv, onSuccess, onCancel }: InstallDialogProps) => {
  const { t } = useT();
  const [detail, setDetail] = useState<SmitheryServerDetail | null>(null);
  const [loadingDetail, setLoadingDetail] = useState(true);
  const [detailError, setDetailError] = useState<string | null>(null);

  const [envValues, setEnvValues] = useState<Record<string, string>>({});
  const [showEnv, setShowEnv] = useState<Record<string, boolean>>({});
  const [configJson, setConfigJson] = useState('');

  const [installing, setInstalling] = useState(false);
  const [installError, setInstallError] = useState<string | null>(null);

  // Track the latest qualifiedName seen by the effect to guard against stale
  // async responses when qualifiedName changes or the component unmounts.
  const latestQualifiedNameRef = useRef(qualifiedName);

  // Fetch server detail on mount or when qualifiedName changes.
  useEffect(() => {
    latestQualifiedNameRef.current = qualifiedName;
    setLoadingDetail(true);
    setDetailError(null);
    log('fetching detail for %s', qualifiedName);
    const requestedName = qualifiedName;
    mcpClientsApi
      .registryGet(qualifiedName)
      .then(d => {
        // Discard response if a newer request has already been issued.
        if (latestQualifiedNameRef.current !== requestedName) {
          log('discarding stale detail response for %s', requestedName);
          return;
        }
        setDetail(d);
        // Pre-fill env values from prop (suggested by config assistant) or empty.
        const initial: Record<string, string> = {};
        for (const key of d.required_env_keys ?? []) {
          initial[key] = prefillEnv?.[key] ?? '';
        }
        setEnvValues(initial);
        log('detail loaded, required_env_keys=%o', d.required_env_keys);
      })
      .catch(err => {
        if (latestQualifiedNameRef.current !== requestedName) return;
        const msg = err instanceof Error ? err.message : t('mcp.install.failedDetail');
        log('detail error: %s', msg);
        setDetailError(msg);
      })
      .finally(() => {
        if (latestQualifiedNameRef.current === requestedName) {
          setLoadingDetail(false);
        }
      });
  }, [qualifiedName, prefillEnv, t]);

  const toggleShowEnv = useCallback((key: string) => {
    setShowEnv(prev => ({ ...prev, [key]: !prev[key] }));
  }, []);

  const handleEnvChange = useCallback((key: string, value: string) => {
    setEnvValues(prev => ({ ...prev, [key]: value }));
  }, []);

  const handleInstall = useCallback(async () => {
    if (!detail) return;

    // Validate required keys are filled.
    for (const key of detail.required_env_keys ?? []) {
      if (!envValues[key]?.trim()) {
        setInstallError(t('mcp.install.missingRequired').replace('{key}', key));
        return;
      }
    }

    // Parse optional JSON config.
    let parsedConfig: unknown = undefined;
    if (configJson.trim()) {
      try {
        parsedConfig = JSON.parse(configJson.trim());
      } catch {
        setInstallError(t('mcp.install.invalidJson'));
        return;
      }
    }

    setInstalling(true);
    setInstallError(null);
    log('installing %s', qualifiedName);

    try {
      const server = await mcpClientsApi.install({
        qualified_name: qualifiedName,
        env: envValues,
        config: parsedConfig,
      });
      log('install success server_id=%s', server.server_id);
      onSuccess(server);
    } catch (err) {
      const msg = err instanceof Error ? err.message : t('mcp.install.failedInstall');
      log('install error: %s', msg);
      setInstallError(msg);
    } finally {
      setInstalling(false);
    }
  }, [detail, envValues, configJson, qualifiedName, onSuccess, t]);

  if (loadingDetail) {
    return (
      <div className="py-10 text-center text-sm text-stone-400 dark:text-neutral-500">
        {t('mcp.install.loadingDetail')}
      </div>
    );
  }

  if (detailError) {
    return (
      <div className="space-y-3">
        <div className="rounded-lg border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-4 py-3 text-sm text-coral-700 dark:text-coral-300">
          {detailError}
        </div>
        <button
          type="button"
          onClick={onCancel}
          className="text-sm text-stone-500 dark:text-neutral-400 hover:underline">
          {t('mcp.install.back')}
        </button>
      </div>
    );
  }

  if (!detail) return null;

  return (
    <div className="space-y-4">
      {/* Header */}
      <div className="flex items-start gap-3">
        {detail.icon_url ? (
          <img
            src={detail.icon_url}
            alt=""
            className="w-10 h-10 rounded shrink-0 object-contain bg-white dark:bg-neutral-900 border border-stone-100 dark:border-neutral-800"
          />
        ) : (
          <div className="w-10 h-10 rounded shrink-0 bg-primary-100 dark:bg-primary-500/20 flex items-center justify-center text-lg">
            🔌
          </div>
        )}
        <div>
          <h3 className="text-base font-semibold text-stone-900 dark:text-neutral-100">
            {t('mcp.install.title').replace('{name}', detail.display_name)}
          </h3>
          {detail.description && (
            <p className="text-xs text-stone-500 dark:text-neutral-400 mt-0.5">
              {detail.description}
            </p>
          )}
        </div>
      </div>

      {/* Env var inputs */}
      {(detail.required_env_keys ?? []).length > 0 && (
        <div className="space-y-2">
          <p className="text-xs font-medium text-stone-700 dark:text-neutral-300">
            {t('mcp.install.requiredEnv')}
          </p>
          {detail.required_env_keys!.map(key => (
            <div key={key} className="space-y-1">
              <label
                htmlFor={`env-${key}`}
                className="block text-xs font-medium text-stone-600 dark:text-neutral-400">
                {key}
              </label>
              <div className="flex gap-2">
                <input
                  id={`env-${key}`}
                  type={showEnv[key] ? 'text' : 'password'}
                  value={envValues[key] ?? ''}
                  onChange={e => handleEnvChange(key, e.target.value)}
                  placeholder={t('mcp.install.enterValue').replace('{key}', key)}
                  disabled={installing}
                  className="flex-1 rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-1.5 text-sm text-stone-800 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:outline-none focus:ring-2 focus:ring-primary-500/40 disabled:opacity-50"
                />
                <button
                  type="button"
                  onClick={() => toggleShowEnv(key)}
                  disabled={installing}
                  className="shrink-0 rounded-lg border border-stone-200 dark:border-neutral-700 px-2 py-1 text-xs text-stone-500 dark:text-neutral-400 hover:border-stone-300 dark:hover:border-neutral-600 disabled:opacity-50">
                  {showEnv[key] ? t('mcp.install.hide') : t('mcp.install.show')}
                </button>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Optional JSON config */}
      <div className="space-y-1">
        <label
          htmlFor="mcp-config-json"
          className="block text-xs font-medium text-stone-600 dark:text-neutral-400">
          {t('mcp.install.configLabel')}
        </label>
        <textarea
          id="mcp-config-json"
          value={configJson}
          onChange={e => setConfigJson(e.target.value)}
          disabled={installing}
          rows={4}
          placeholder={t('mcp.install.configPlaceholder')}
          className="w-full rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm font-mono text-stone-800 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:outline-none focus:ring-2 focus:ring-primary-500/40 disabled:opacity-50 resize-y"
        />
      </div>

      {/* Error */}
      {installError && (
        <div className="rounded-lg border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-4 py-3 text-sm text-coral-700 dark:text-coral-300">
          {installError}
        </div>
      )}

      {/* Actions */}
      <div className="flex gap-2">
        <button
          type="button"
          disabled={installing}
          onClick={() => void handleInstall()}
          className="rounded-lg bg-primary-500 px-4 py-2 text-sm font-medium text-white hover:bg-primary-600 disabled:opacity-50 transition-colors">
          {installing ? t('mcp.install.installing') : t('mcp.install.button')}
        </button>
        <button
          type="button"
          disabled={installing}
          onClick={onCancel}
          className="rounded-lg border border-stone-200 dark:border-neutral-700 px-4 py-2 text-sm font-medium text-stone-600 dark:text-neutral-300 hover:border-stone-300 dark:hover:border-neutral-600 disabled:opacity-50">
          {t('common.cancel')}
        </button>
      </div>
    </div>
  );
};

export default InstallDialog;
