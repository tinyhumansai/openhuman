/**
 * Inline "open your memory vault in Obsidian" control for the Memory tab.
 *
 * The vault is the on-disk `<workspace>/memory_tree/content/` folder. Opening
 * it uses an `obsidian://open?path=…` deep link — but that scheme only
 * resolves folders Obsidian has already registered as a vault (it cannot
 * register one, and a `.obsidian/` folder on disk is not enough). So a
 * first-time click would otherwise land on Obsidian's *"Unable to find a vault
 * for the URL"* error.
 *
 * Flow (progressive disclosure):
 *  - Click → check registration via `memory_tree_obsidian_vault_status`.
 *  - Registered → fire the deep link (success toast). Done.
 *  - Not registered → expand inline guidance instead of firing a doomed link:
 *      Reveal Folder · Open anyway · Install Obsidian · Advanced ▸ config-dir
 *      override.
 *
 * Detection is best-effort — Obsidian can live somewhere we can't probe
 * (Flatpak/Snap/portable). "Open anyway" and the config-dir override are the
 * escape hatches for that case; a false "not registered" never blocks the user.
 */
import { useCallback, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { ToastNotification } from '../../types/intelligence';
import { openUrl } from '../../utils/openUrl';
import { memoryTreeObsidianVaultStatus } from '../../utils/tauriCommands';
import {
  resolveWorkspaceAbsolutePath,
  revealWorkspacePath,
} from '../../utils/tauriCommands/workspacePaths';
import { MEMORY_CONTENT_WORKSPACE_PATH } from './memoryWorkspacePaths';

/** localStorage key for the optional Obsidian config-dir override. */
const CONFIG_DIR_KEY = 'openhuman.obsidian.configDir';
const OBSIDIAN_DOWNLOAD_URL = 'https://obsidian.md/download';

interface ObsidianVaultSectionProps {
  /** Absolute path to `<workspace>/memory_tree/content/` (from graph export). */
  contentRootAbs: string;
  onToast?: (toast: Omit<ToastNotification, 'id'>) => void;
}

function readConfigDirOverride(): string {
  try {
    return localStorage.getItem(CONFIG_DIR_KEY) ?? '';
  } catch {
    return '';
  }
}

export function ObsidianVaultSection({ contentRootAbs, onToast }: ObsidianVaultSectionProps) {
  const { t } = useT();
  const [checking, setChecking] = useState(false);
  const [expanded, setExpanded] = useState(false);
  // null = not probed yet / probe failed; otherwise last `config_found`.
  const [configFound, setConfigFound] = useState<boolean | null>(null);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [configDir, setConfigDir] = useState<string>(readConfigDirOverride);

  /**
   * Build + fire the `obsidian://` deep link.
   *
   * The absolute path is resolved through the shared workspace-link layer
   * (`resolve_workspace_absolute_path` Tauri command) rather than reused from
   * the `contentRootAbs` prop — this routes Obsidian deep links through the
   * same canonicalize + workspace-containment guard as `open_workspace_path`
   * and `preview_workspace_text` (see issue #2492 / #2476).
   *
   * Resolves to an error or null.
   */
  const fireDeepLink = useCallback(async (): Promise<unknown | null> => {
    console.debug('[ui-flow][obsidian-vault] firing deep link');
    try {
      const absolutePath = await resolveWorkspaceAbsolutePath(MEMORY_CONTENT_WORKSPACE_PATH);
      const url = `obsidian://open?path=${encodeURIComponent(absolutePath)}`;
      await openUrl(url);
      return null;
    } catch (err) {
      console.error('[ui-flow][obsidian-vault] resolve/open failed', err);
      return err;
    }
  }, []);

  const reveal = useCallback(() => {
    void (async () => {
      try {
        await revealWorkspacePath(MEMORY_CONTENT_WORKSPACE_PATH);
      } catch (err) {
        console.error('[ui-flow][obsidian-vault] revealWorkspacePath failed', err);
        onToast?.({
          type: 'error',
          title: t('workspace.revealVaultFailed'),
          message: err instanceof Error ? err.message : String(err),
        });
      }
    })();
  }, [onToast, t]);

  const toastOpenOutcome = useCallback(
    (err: unknown | null) => {
      onToast?.(
        err === null
          ? {
              type: 'info',
              title: t('workspace.openingVaultTitle'),
              message: `${t('workspace.openingVaultMessage')} ${contentRootAbs}`,
              action: { label: t('workspace.revealFolder'), handler: reveal },
            }
          : {
              type: 'error',
              title: t('workspace.openVaultFailedTitle'),
              message: `${t('workspace.openVaultFailedMessage')} ${contentRootAbs}`,
              action: { label: t('workspace.revealFolder'), handler: reveal },
            }
      );
    },
    [onToast, t, contentRootAbs, reveal]
  );

  /** Fire the deep link unconditionally — the "Open anyway" escape hatch. */
  const openAnyway = useCallback(() => {
    void (async () => {
      toastOpenOutcome(await fireDeepLink());
    })();
  }, [fireDeepLink, toastOpenOutcome]);

  const installObsidian = useCallback(() => {
    // https URL → openUrl falls back to window.open if the IPC bridge isn't
    // ready, so this normally reaches the download page. Swallow + log any
    // rejection so it can't surface as an unhandled promise rejection.
    void (async () => {
      try {
        await openUrl(OBSIDIAN_DOWNLOAD_URL);
      } catch (err) {
        console.error('[ui-flow][obsidian-vault] openUrl(download) failed', err);
      }
    })();
  }, []);

  const handleViewVault = useCallback(() => {
    void (async () => {
      setChecking(true);
      try {
        const override = configDir.trim();
        const status = await memoryTreeObsidianVaultStatus(override || undefined);
        console.debug(
          '[ui-flow][obsidian-vault] status registered=%s config_found=%s',
          status.registered,
          status.config_found
        );
        setConfigFound(status.config_found);

        if (status.registered) {
          // Known vault — open it directly.
          const err = await fireDeepLink();
          toastOpenOutcome(err);
          // Registered but the IPC/scheme still failed → surface guidance;
          // on success collapse any stale panel from a prior failed check.
          setExpanded(err !== null);
          return;
        }

        // Not registered — guide the user instead of firing a doomed link.
        setExpanded(true);
      } catch (err) {
        // Detection itself failed — degrade gracefully to the guidance panel.
        console.error('[ui-flow][obsidian-vault] status check failed', err);
        setConfigFound(null);
        setExpanded(true);
      } finally {
        setChecking(false);
      }
    })();
  }, [configDir, fireDeepLink, toastOpenOutcome]);

  const saveConfigDir = useCallback(() => {
    const trimmed = configDir.trim();
    try {
      if (trimmed) localStorage.setItem(CONFIG_DIR_KEY, trimmed);
      else localStorage.removeItem(CONFIG_DIR_KEY);
    } catch (err) {
      console.warn('[ui-flow][obsidian-vault] persist config dir failed', err);
    }
    // Re-run the check with the new override applied.
    handleViewVault();
  }, [configDir, handleViewVault]);

  const helpText =
    configFound === false
      ? t('workspace.obsidianNotFoundHelp')
      : t('workspace.vaultNotRegisteredHelp');

  return (
    <div className="flex flex-col items-end gap-2" data-testid="obsidian-vault-section">
      <button
        type="button"
        onClick={handleViewVault}
        disabled={checking}
        data-testid="memory-open-in-obsidian"
        className="inline-flex items-center gap-1.5 rounded-lg
                   border border-stone-200 bg-white px-3 py-1.5 text-xs font-semibold
                   text-stone-700 shadow-sm transition-colors hover:bg-stone-50
                   disabled:cursor-not-allowed disabled:opacity-50
                   focus:outline-none focus:ring-2 focus:ring-stone-200
                   dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-200
                   dark:hover:bg-neutral-800 dark:focus:ring-neutral-700"
        title={`obsidian://open?path=${contentRootAbs}`}>
        <ExternalLinkIcon />
        {checking ? t('workspace.checkingVault') : t('workspace.viewVault')}
      </button>

      {expanded && (
        <div
          data-testid="obsidian-vault-guidance"
          className="w-full max-w-xl rounded-lg border border-violet-200 bg-violet-50 p-4
                     text-sm dark:border-violet-500/30 dark:bg-violet-500/10">
          <p className="text-neutral-700 dark:text-neutral-200">{helpText}</p>

          <code
            className="mt-2 block break-all rounded bg-white/70 px-2 py-1 font-mono text-xs
                       text-neutral-600 dark:bg-neutral-900/60 dark:text-neutral-300"
            data-testid="obsidian-vault-path">
            {contentRootAbs}
          </code>

          <div className="mt-3 flex flex-wrap gap-2">
            <button
              type="button"
              onClick={reveal}
              data-testid="obsidian-reveal"
              className="rounded-md border border-neutral-300 bg-white px-3 py-1.5 text-xs font-semibold
                         text-neutral-700 hover:bg-neutral-50 dark:border-neutral-600
                         dark:bg-neutral-800 dark:text-neutral-200">
              {t('workspace.revealFolder')}
            </button>
            <button
              type="button"
              onClick={openAnyway}
              data-testid="obsidian-open-anyway"
              className="rounded-md border border-violet-300 bg-white px-3 py-1.5 text-xs font-semibold
                         text-violet-700 hover:bg-violet-50 dark:border-violet-500/40
                         dark:bg-neutral-800 dark:text-violet-300">
              {t('workspace.openAnyway')}
            </button>
            <button
              type="button"
              onClick={installObsidian}
              data-testid="obsidian-install"
              className="rounded-md border border-neutral-300 bg-white px-3 py-1.5 text-xs font-semibold
                         text-neutral-700 hover:bg-neutral-50 dark:border-neutral-600
                         dark:bg-neutral-800 dark:text-neutral-200">
              {t('workspace.installObsidian')}
            </button>
          </div>

          <button
            type="button"
            onClick={() => setShowAdvanced(v => !v)}
            data-testid="obsidian-advanced-toggle"
            className="mt-3 text-xs font-medium text-violet-600 hover:underline dark:text-violet-300">
            {t('workspace.obsidianAdvanced')}
          </button>

          {showAdvanced && (
            <div className="mt-2 space-y-1.5">
              <label
                htmlFor="obsidian-config-dir"
                className="block text-xs font-medium text-neutral-600 dark:text-neutral-300">
                {t('workspace.obsidianConfigDirLabel')}
              </label>
              <div className="flex flex-wrap items-center gap-2">
                <input
                  id="obsidian-config-dir"
                  type="text"
                  value={configDir}
                  onChange={e => setConfigDir(e.target.value)}
                  placeholder={t('workspace.obsidianConfigDirPlaceholder')}
                  spellCheck={false}
                  data-testid="obsidian-config-dir-input"
                  className="flex-1 rounded-md border border-neutral-300 bg-white px-2 py-1 font-mono text-xs
                             text-neutral-800 focus:outline-none focus:ring-1 focus:ring-violet-300
                             dark:border-neutral-600 dark:bg-neutral-900 dark:text-neutral-100"
                />
                <button
                  type="button"
                  onClick={saveConfigDir}
                  disabled={checking}
                  data-testid="obsidian-config-dir-save"
                  className="rounded-md bg-violet-500 px-3 py-1 text-xs font-semibold text-white
                             hover:bg-violet-600 disabled:opacity-50">
                  {t('common.save')}
                </button>
              </div>
              <p className="text-xs text-neutral-500 dark:text-neutral-400">
                {t('workspace.obsidianConfigDirHint')}
              </p>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function ExternalLinkIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true">
      <path d="M14 3h7v7" />
      <path d="M10 14L21 3" />
      <path d="M21 14v7H3V3h7" />
    </svg>
  );
}
