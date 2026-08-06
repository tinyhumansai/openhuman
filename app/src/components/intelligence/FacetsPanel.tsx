/**
 * FacetsPanel — Brain > Profile.
 *
 * Lists learned personalization facets (Active + Provisional), lets the user
 * pin / unpin / forget them, rebuild the ambient cache, and toggle
 * `learning.enabled`.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { LuPin, LuPinOff, LuRefreshCw, LuTrash2 } from 'react-icons/lu';

import { useT } from '../../lib/i18n/I18nContext';
import { learningApi, type LearningFacet } from '../../services/api/learningApi';
import Button from '../ui/Button';

const cardClass = 'rounded-lg border border-line bg-surface p-4';

const CLASS_ORDER = ['style', 'identity', 'tooling', 'veto', 'goal', 'channel'] as const;

function classOf(facet: LearningFacet): string {
  if (facet.class) return facet.class;
  const i = facet.key.indexOf('/');
  return i > 0 ? facet.key.slice(0, i) : 'other';
}

function displayKey(facet: LearningFacet): string {
  const i = facet.key.indexOf('/');
  return i > 0 ? facet.key.slice(i + 1) : facet.key;
}

export default function FacetsPanel() {
  const { t } = useT();
  const [facets, setFacets] = useState<LearningFacet[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [rebuilding, setRebuilding] = useState(false);
  const [learningEnabled, setLearningEnabled] = useState(false);
  const [toggling, setToggling] = useState(false);

  const mountedRef = useRef(true);

  const load = useCallback(async () => {
    setError(null);
    try {
      const [list, settings] = await Promise.all([
        learningApi.listFacets(),
        learningApi.getSettings(),
      ]);
      if (mountedRef.current) {
        setFacets(list);
        setLearningEnabled(settings.enabled);
      }
    } catch (err) {
      if (mountedRef.current) setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    void load();
    return () => {
      mountedRef.current = false;
    };
  }, [load]);

  const grouped = useMemo(() => {
    const map = new Map<string, LearningFacet[]>();
    for (const f of facets) {
      const cls = classOf(f);
      const bucket = map.get(cls) ?? [];
      bucket.push(f);
      map.set(cls, bucket);
    }
    for (const bucket of map.values()) {
      bucket.sort((a, b) => b.stability - a.stability || a.key.localeCompare(b.key));
    }
    const ordered: { className: string; items: LearningFacet[] }[] = [];
    for (const cls of CLASS_ORDER) {
      const items = map.get(cls);
      if (items?.length) ordered.push({ className: cls, items });
      map.delete(cls);
    }
    for (const [cls, items] of [...map.entries()].sort(([a], [b]) => a.localeCompare(b))) {
      ordered.push({ className: cls, items });
    }
    return ordered;
  }, [facets]);

  const runAction = useCallback(
    async (key: string, action: () => Promise<void>) => {
      setActionError(null);
      setBusyKey(key);
      try {
        await action();
        const list = await learningApi.listFacets();
        if (mountedRef.current) {
          setFacets(list);
          setError(null);
        }
      } catch (err) {
        if (mountedRef.current) {
          setActionError(err instanceof Error ? err.message : t('brain.profile.actionError'));
        }
      } finally {
        if (mountedRef.current) setBusyKey(null);
      }
    },
    [t]
  );

  const handleRebuild = useCallback(async () => {
    setActionError(null);
    setRebuilding(true);
    try {
      await learningApi.rebuildCache();
      const list = await learningApi.listFacets();
      if (mountedRef.current) {
        setFacets(list);
        setError(null);
      }
    } catch (err) {
      if (mountedRef.current) {
        setActionError(err instanceof Error ? err.message : t('brain.profile.actionError'));
      }
    } finally {
      if (mountedRef.current) setRebuilding(false);
    }
  }, [t]);

  const handleToggleLearning = useCallback(async () => {
    setActionError(null);
    setToggling(true);
    const next = !learningEnabled;
    try {
      const settings = await learningApi.updateSettings(next);
      if (mountedRef.current) {
        setLearningEnabled(settings.enabled);
        setError(null);
      }
    } catch (err) {
      if (mountedRef.current) {
        setActionError(err instanceof Error ? err.message : t('brain.profile.actionError'));
      }
    } finally {
      if (mountedRef.current) setToggling(false);
    }
  }, [learningEnabled, t]);

  if (loading) {
    return (
      <div className={`${cardClass} text-sm text-content-muted`} data-testid="facets-panel-loading">
        {t('brain.profile.loading')}
      </div>
    );
  }

  if (error) {
    return (
      <div
        className={`${cardClass} text-sm text-coral-600`}
        role="alert"
        data-testid="facets-panel-error">
        {error}
      </div>
    );
  }

  return (
    <div className="space-y-3 animate-fade-up" data-testid="facets-panel">
      <div className={cardClass}>
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0">
            <h2 className="text-sm font-semibold text-content">{t('brain.profile.title')}</h2>
            <p className="mt-0.5 text-xs text-content-muted">{t('brain.profile.description')}</p>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <label className="flex items-center gap-2 text-xs text-content-secondary">
              <input
                type="checkbox"
                className="h-3.5 w-3.5 rounded border-line"
                checked={learningEnabled}
                disabled={toggling}
                onChange={() => void handleToggleLearning()}
                data-testid="learning-enabled-toggle"
              />
              {t('brain.profile.learningEnabled')}
            </label>
            <Button
              type="button"
              variant="secondary"
              size="sm"
              onClick={() => void handleRebuild()}
              disabled={rebuilding}
              data-testid="facets-rebuild">
              <LuRefreshCw className={`mr-1.5 h-3.5 w-3.5 ${rebuilding ? 'animate-spin' : ''}`} />
              {rebuilding ? t('brain.profile.rebuilding') : t('brain.profile.rebuild')}
            </Button>
          </div>
        </div>
        {!learningEnabled && (
          <p
            className="mt-3 text-xs text-amber-700 dark:text-amber-300"
            data-testid="learning-off-hint">
            {t('brain.profile.learningOffHint')}
          </p>
        )}
        {actionError && (
          <p className="mt-3 text-xs text-coral-600" role="alert">
            {actionError}
          </p>
        )}
      </div>

      {facets.length === 0 ? (
        <div className={`${cardClass} text-sm text-content-muted`} data-testid="facets-empty">
          {t('brain.profile.empty')}
        </div>
      ) : (
        grouped.map(({ className, items }) => (
          <div key={className} className={cardClass} data-testid={`facets-class-${className}`}>
            <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-content-secondary">
              {t(`brain.profile.class.${className}`, className)}
            </h3>
            <ul className="divide-y divide-line">
              {items.map(facet => {
                const pinned = facet.user_state === 'pinned';
                const busy = busyKey === facet.key;
                return (
                  <li
                    key={facet.key}
                    className="flex items-start justify-between gap-3 py-2.5"
                    data-testid={`facet-row-${facet.key}`}>
                    <div className="min-w-0">
                      <div className="text-sm font-medium text-content">
                        {displayKey(facet)}
                        {pinned && (
                          <span className="ml-1.5 text-xs font-normal text-content-muted">
                            {t('brain.profile.pinned')}
                          </span>
                        )}
                      </div>
                      <div className="mt-0.5 break-words text-xs text-content-secondary">
                        {facet.value}
                      </div>
                      <div className="mt-0.5 text-[11px] text-content-muted">
                        {t('brain.profile.meta')
                          .replace('{{state}}', facet.state)
                          .replace('{{stability}}', facet.stability.toFixed(2))}
                      </div>
                    </div>
                    <div className="flex shrink-0 items-center gap-1">
                      <Button
                        type="button"
                        variant="tertiary"
                        size="sm"
                        disabled={busy}
                        aria-label={pinned ? t('brain.profile.unpin') : t('brain.profile.pin')}
                        onClick={() =>
                          void runAction(facet.key, () =>
                            pinned
                              ? learningApi.unpinFacet(facet.key)
                              : learningApi.pinFacet(facet.key)
                          )
                        }
                        data-testid={`facet-pin-${facet.key}`}>
                        {pinned ? (
                          <LuPinOff className="h-3.5 w-3.5" />
                        ) : (
                          <LuPin className="h-3.5 w-3.5" />
                        )}
                      </Button>
                      <Button
                        type="button"
                        variant="tertiary"
                        size="sm"
                        disabled={busy}
                        aria-label={t('brain.profile.forget')}
                        onClick={() =>
                          void runAction(facet.key, () => learningApi.forgetFacet(facet.key))
                        }
                        data-testid={`facet-forget-${facet.key}`}>
                        <LuTrash2 className="h-3.5 w-3.5" />
                      </Button>
                    </div>
                  </li>
                );
              })}
            </ul>
          </div>
        ))
      )}
    </div>
  );
}
