/**
 * "Why kept" score bars — SVG-rendered (not CSS divs) for crisp pixel
 * alignment regardless of zoom or DPR.
 */
import { useT } from '../../lib/i18n/I18nContext';
import type { ScoreBreakdown } from '../../utils/tauriCommands';

interface MemoryChunkScoreBarsProps {
  breakdown: ScoreBreakdown;
}

const TRACK_WIDTH = 200;
const TRACK_HEIGHT = 8;

function clamp01(v: number): number {
  if (Number.isNaN(v)) return 0;
  return Math.max(0, Math.min(1, v));
}

export function MemoryChunkScoreBars({ breakdown }: MemoryChunkScoreBarsProps) {
  const { t } = useT();
  return (
    <section data-testid="memory-chunk-scorebars">
      <h3 className="mw-whykept-heading">{t('intelligence.memoryChunk.scoreBars.heading')}</h3>
      <div>
        {breakdown.signals.map(sig => {
          const pct = clamp01(sig.value);
          return (
            <div key={sig.name} className="mw-scorebar-row">
              <span className="mw-scorebar-label">{sig.name}</span>
              <svg
                width="100%"
                viewBox={`0 0 ${TRACK_WIDTH} ${TRACK_HEIGHT}`}
                preserveAspectRatio="none"
                role="img"
                aria-label={t('intelligence.memoryChunk.scoreBars.ariaScore')
                  .replace('{name}', sig.name)
                  .replace('{pct}', (pct * 100).toFixed(0))}>
                <rect
                  x={0}
                  y={0}
                  width={TRACK_WIDTH}
                  height={TRACK_HEIGHT}
                  rx={2}
                  fill="var(--paper-recessed)"
                />
                <rect
                  x={0}
                  y={0}
                  width={pct * TRACK_WIDTH}
                  height={TRACK_HEIGHT}
                  rx={2}
                  fill="var(--sage)"
                />
              </svg>
              <span className="mw-scorebar-value">{pct.toFixed(2)}</span>
            </div>
          );
        })}
      </div>
      <div className="mw-scorebar-threshold">
        ───{' '}
        {breakdown.kept
          ? t('intelligence.memoryChunk.scoreBars.kept')
          : t('intelligence.memoryChunk.scoreBars.dropped')}{' '}
        {t('intelligence.memoryChunk.scoreBars.atThreshold').replace(
          '{threshold}',
          breakdown.threshold.toFixed(2)
        )}{' '}
        ───
      </div>
    </section>
  );
}
