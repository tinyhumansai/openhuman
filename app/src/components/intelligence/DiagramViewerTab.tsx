import { useCallback, useEffect, useMemo, useState } from 'react';
import { LuImage, LuRefreshCw } from 'react-icons/lu';

import { useT } from '../../lib/i18n/I18nContext';
import {
  type DiagramViewerSettings,
  openhumanGetDashboardSettings,
} from '../../utils/tauriCommands/config';
import Button from '../ui/Button';

const DEFAULT_SETTINGS: DiagramViewerSettings = {
  enabled: true,
  source_url: 'http://localhost:8787/workspace/diagrams/latest.png',
  refresh_interval_seconds: 10,
};

type ImageState = 'idle' | 'loaded' | 'error';

function normalizeSettings(
  settings?: Partial<DiagramViewerSettings> | null
): DiagramViewerSettings {
  const sourceUrl = settings?.source_url?.trim() || DEFAULT_SETTINGS.source_url;
  const refreshInterval = Number(settings?.refresh_interval_seconds);

  return {
    enabled: settings?.enabled ?? DEFAULT_SETTINGS.enabled,
    source_url: sourceUrl,
    refresh_interval_seconds:
      Number.isFinite(refreshInterval) && refreshInterval > 0
        ? Math.round(refreshInterval)
        : DEFAULT_SETTINGS.refresh_interval_seconds,
  };
}

export function buildDiagramImageUrl(sourceUrl: string, refreshKey: number): string {
  try {
    const url = new URL(sourceUrl);
    url.searchParams.set('openhuman_refresh', String(refreshKey));
    return url.toString();
  } catch {
    const separator = sourceUrl.includes('?') ? '&' : '?';
    return `${sourceUrl}${separator}openhuman_refresh=${refreshKey}`;
  }
}

export default function DiagramViewerTab() {
  const { t } = useT();
  const [settings, setSettings] = useState<DiagramViewerSettings>(DEFAULT_SETTINGS);
  const [refreshKey, setRefreshKey] = useState(0);
  const [imageState, setImageState] = useState<ImageState>('idle');

  useEffect(() => {
    let alive = true;

    openhumanGetDashboardSettings()
      .then(response => {
        if (!alive) return;
        setSettings(normalizeSettings(response.result.diagram_viewer));
      })
      .catch(() => {
        if (!alive) return;
        setSettings(DEFAULT_SETTINGS);
      });

    return () => {
      alive = false;
    };
  }, []);

  const refreshDiagram = useCallback(() => {
    setImageState('idle');
    setRefreshKey(prev => prev + 1);
  }, []);

  useEffect(() => {
    if (!settings.enabled || settings.refresh_interval_seconds <= 0) return undefined;

    const interval = window.setInterval(refreshDiagram, settings.refresh_interval_seconds * 1000);
    return () => window.clearInterval(interval);
  }, [refreshDiagram, settings.enabled, settings.refresh_interval_seconds]);

  const sourceUrl = settings.source_url.trim();
  const imageUrl = useMemo(
    () => (sourceUrl ? buildDiagramImageUrl(sourceUrl, refreshKey) : ''),
    [refreshKey, sourceUrl]
  );

  const showImage = settings.enabled && sourceUrl.length > 0 && imageState !== 'error';
  const showEmptyState = !settings.enabled || sourceUrl.length === 0 || imageState === 'error';

  return (
    <section className="space-y-5" aria-labelledby="diagram-viewer-title">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 id="diagram-viewer-title" className="text-lg font-semibold text-content">
            {t('intelligence.diagram.title')}
          </h2>
          <p className="mt-1 text-sm text-content-muted">{t('intelligence.diagram.description')}</p>
        </div>
        <Button
          variant="secondary"
          size="md"
          onClick={refreshDiagram}
          leadingIcon={<LuRefreshCw aria-hidden="true" className="h-4 w-4" />}
          aria-label={t('intelligence.diagram.refreshAria')}>
          {t('intelligence.diagram.refresh')}
        </Button>
      </div>

      {showEmptyState && (
        <div className="flex min-h-72 flex-col items-center justify-center gap-3 rounded-lg border border-dashed border-line-strong bg-surface-muted px-6 py-10 text-center dark:bg-surface-canvas/60">
          <div className="flex h-12 w-12 items-center justify-center rounded-full bg-primary-50 text-primary-600 dark:bg-primary-500/10 dark:text-primary-300">
            <LuImage aria-hidden="true" className="h-6 w-6" />
          </div>
          <div>
            <h3 className="text-sm font-semibold text-content">
              {t('intelligence.diagram.emptyTitle')}
            </h3>
            <p className="mt-1 max-w-md text-sm text-content-muted">
              {t('intelligence.diagram.emptyDescription')}
            </p>
          </div>
          <div className="flex max-w-full flex-col gap-2">
            <code className="max-w-full overflow-x-auto rounded-md bg-surface px-3 py-2 text-xs text-content-secondary">
              {t('intelligence.diagram.skillInstallCommand')}
            </code>
            <code className="max-w-full overflow-x-auto rounded-md bg-surface px-3 py-2 text-xs text-content-secondary">
              {t('intelligence.diagram.promptExample')}
            </code>
          </div>
        </div>
      )}

      {showImage && (
        <figure className="space-y-3">
          <img
            key={imageUrl}
            src={imageUrl}
            alt={t('intelligence.diagram.imageAlt')}
            className="block w-full rounded-lg border border-line bg-surface object-contain dark:bg-surface-canvas"
            onLoad={() => setImageState('loaded')}
            onError={() => setImageState('error')}
          />
          <figcaption className="flex flex-wrap items-center justify-between gap-2 text-xs text-content-muted">
            <span>
              {t('intelligence.diagram.refreshesEvery').replace(
                '{seconds}',
                String(settings.refresh_interval_seconds)
              )}
            </span>
            <span className="max-w-full truncate">{sourceUrl}</span>
          </figcaption>
        </figure>
      )}
    </section>
  );
}
