import { useCallback, useEffect, useState } from 'react';

import {
  disableTrigger,
  enableTrigger,
  listAvailableTriggers,
  listTriggers,
} from '../../lib/composio/composioApi';
import { formatTriggerLabel } from '../../lib/composio/formatters';
import type { ComposioActiveTrigger, ComposioAvailableTrigger } from '../../lib/composio/types';
import { useT } from '../../lib/i18n/I18nContext';

/**
 * Stable signature for matching an `AvailableTrigger` to an
 * `ActiveTrigger`. Static toolkits key by slug; GitHub per-repo
 * triggers key by `slug::owner/repo` to disambiguate the same slug
 * across repos.
 */
export function triggerSignature(
  slug: string,
  scope: 'static' | 'github_repo',
  config?: { owner?: string; repo?: string }
): string {
  if (scope === 'github_repo' && config?.owner && config?.repo) {
    return `${slug.toUpperCase()}::${config.owner.toLowerCase()}/${config.repo.toLowerCase()}`;
  }
  return slug.toUpperCase();
}

export function activeTriggerSignature(t: ComposioActiveTrigger): string {
  const cfg = t.triggerConfig ?? {};
  const owner = typeof cfg.owner === 'string' ? cfg.owner : undefined;
  const repo = typeof cfg.repo === 'string' ? cfg.repo : undefined;
  if (owner && repo) {
    return `${t.slug.toUpperCase()}::${owner.toLowerCase()}/${repo.toLowerCase()}`;
  }
  return t.slug.toUpperCase();
}

export interface TriggerTogglesProps {
  toolkitSlug: string;
  toolkitName: string;
  connectionId: string;
}

export default function TriggerToggles({
  toolkitSlug,
  toolkitName,
  connectionId,
}: TriggerTogglesProps) {
  const { t } = useT();
  const [available, setAvailable] = useState<ComposioAvailableTrigger[] | null>(null);
  const [activeBySignature, setActiveBySignature] = useState<Map<string, ComposioActiveTrigger>>(
    new Map()
  );
  const [loadError, setLoadError] = useState<string | null>(null);
  const [pendingSignature, setPendingSignature] = useState<string | null>(null);
  const [rowError, setRowError] = useState<string | null>(null);

  // Load both lists in parallel on mount / when connection changes.
  useEffect(() => {
    let cancelled = false;
    setAvailable(null);
    setActiveBySignature(new Map());
    setLoadError(null);
    void (async () => {
      try {
        const [avail, active] = await Promise.all([
          listAvailableTriggers(toolkitSlug, connectionId),
          listTriggers(toolkitSlug),
        ]);
        if (cancelled) return;
        setAvailable(avail.triggers);
        const map = new Map<string, ComposioActiveTrigger>();
        for (const t of active.triggers) {
          if (t.connectionId && t.connectionId !== connectionId) continue;
          map.set(activeTriggerSignature(t), t);
        }
        setActiveBySignature(map);
      } catch (err) {
        if (cancelled) return;
        const msg = err instanceof Error ? err.message : String(err);
        setLoadError(`${t('composio.triggers.loadError')}: ${msg}`);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [toolkitSlug, connectionId]);

  const handleToggle = useCallback(
    async (entry: ComposioAvailableTrigger) => {
      const config = entry.scope === 'github_repo' ? entry.repo : undefined;
      const sig = triggerSignature(entry.slug, entry.scope, config);
      if (pendingSignature) return;
      setPendingSignature(sig);
      setRowError(null);

      const existing = activeBySignature.get(sig);
      try {
        if (existing) {
          await disableTrigger(existing.id);
          setActiveBySignature(prev => {
            const next = new Map(prev);
            next.delete(sig);
            return next;
          });
        } else {
          const triggerConfig =
            entry.scope === 'github_repo' && entry.repo
              ? { owner: entry.repo.owner, repo: entry.repo.repo }
              : entry.defaultConfig;
          const created = await enableTrigger(connectionId, entry.slug, triggerConfig);
          setActiveBySignature(prev => {
            const next = new Map(prev);
            next.set(sig, {
              id: created.triggerId,
              slug: created.slug,
              toolkit: toolkitSlug,
              connectionId: created.connectionId,
              ...(triggerConfig ? { triggerConfig } : {}),
            });
            return next;
          });
        }
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        const actionWord = existing ? t('common.disable') : t('common.enable');
        setRowError(
          t('triggers.toggleFailed')
            .replace('{action}', actionWord)
            .replace(
              '{trigger}',
              formatTriggerLabel(entry.slug, { toolkit: toolkitName || toolkitSlug })
            )
            .replace('{message}', msg)
        );
      } finally {
        setPendingSignature(null);
      }
    },
    [activeBySignature, connectionId, pendingSignature, toolkitSlug]
  );

  if (loadError) {
    return (
      <div className="border-t border-stone-100 dark:border-neutral-800 pt-3 mt-1">
        <p className="text-[11px] text-coral-600">{loadError}</p>
      </div>
    );
  }

  if (available === null) {
    return (
      <div className="border-t border-stone-100 dark:border-neutral-800 pt-3 mt-1">
        <h3 className="text-xs font-semibold text-stone-700 dark:text-neutral-200 uppercase tracking-wide">
          {t('composio.triggers.heading')}
        </h3>
        <p className="mt-1 text-[11px] text-stone-400 dark:text-neutral-500">
          {t('composio.triggers.loading')}
        </p>
      </div>
    );
  }

  if (available.length === 0) {
    return (
      <div className="border-t border-stone-100 dark:border-neutral-800 pt-3 mt-1">
        <h3 className="text-xs font-semibold text-stone-700 dark:text-neutral-200 uppercase tracking-wide">
          {t('composio.triggers.heading')}
        </h3>
        <p className="mt-1 text-[11px] text-stone-400 dark:text-neutral-500">
          {`${t('composio.triggers.noneAvailable')} ${toolkitName}.`}
        </p>
      </div>
    );
  }

  return (
    <div
      className="border-t border-stone-100 dark:border-neutral-800 pt-3 mt-1 space-y-2"
      data-testid="trigger-toggles">
      <div className="flex items-baseline justify-between">
        <h3 className="text-xs font-semibold text-stone-700 dark:text-neutral-200 uppercase tracking-wide">
          {t('composio.triggers.heading')}
        </h3>
        <p className="text-[10px] text-stone-400 dark:text-neutral-500">{`${t('composio.triggers.listenFrom')} ${toolkitName}`}</p>
      </div>
      <ul className="space-y-1.5 max-h-56 overflow-y-auto pr-1">
        {available.map(entry => {
          const config = entry.scope === 'github_repo' ? entry.repo : undefined;
          const sig = triggerSignature(entry.slug, entry.scope, config);
          const enabled = activeBySignature.has(sig);
          const requiresConfig =
            (entry.requiredConfigKeys?.length ?? 0) > 0 && entry.scope === 'static';
          const isPending = pendingSignature === sig;
          const disabled = requiresConfig || pendingSignature !== null;

          const label =
            entry.scope === 'github_repo' && entry.repo
              ? `${entry.repo.owner}/${entry.repo.repo}`
              : formatTriggerLabel(entry.slug, { toolkit: toolkitName || toolkitSlug });
          const sub =
            entry.scope === 'github_repo'
              ? formatTriggerLabel(entry.slug, { toolkit: toolkitName || toolkitSlug })
              : requiresConfig
                ? t('composio.triggers.needsConfiguration')
                : '';
          const action = enabled ? t('common.disable') : t('common.enable');
          const triggerName = formatTriggerLabel(entry.slug, {
            toolkit: toolkitName || toolkitSlug,
          });
          const ariaLabel =
            entry.scope === 'github_repo' && entry.repo
              ? `${action} ${triggerName} for ${entry.repo.owner}/${entry.repo.repo}`
              : `${action} ${triggerName}`;

          return (
            <li
              key={sig}
              data-testid={`trigger-row-${sig}`}
              className="flex items-start justify-between gap-3 rounded-lg px-2 py-1.5 hover:bg-stone-50 dark:hover:bg-neutral-800/60">
              <div className="min-w-0 flex-1">
                <span className="text-sm font-medium text-stone-900 dark:text-neutral-100 break-all">
                  {label}
                </span>
                {sub && (
                  <p className="text-[11px] text-stone-400 dark:text-neutral-500 leading-snug">
                    {sub}
                  </p>
                )}
              </div>
              <button
                type="button"
                role="switch"
                aria-checked={enabled}
                aria-label={ariaLabel}
                disabled={disabled}
                onClick={() => void handleToggle(entry)}
                className={`relative inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1 disabled:cursor-not-allowed disabled:opacity-50 ${
                  enabled ? 'bg-primary-500' : 'bg-stone-300 dark:bg-neutral-700'
                }`}>
                <span
                  className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white dark:bg-neutral-900 shadow transition-transform ${
                    enabled ? 'translate-x-5' : 'translate-x-0.5'
                  } ${isPending ? 'animate-pulse' : ''}`}
                />
              </button>
            </li>
          );
        })}
      </ul>
      {rowError && <p className="text-[11px] text-coral-600">{rowError}</p>}
    </div>
  );
}
