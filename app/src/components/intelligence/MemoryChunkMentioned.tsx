/**
 * "Mentioned" entity list — the marginalia of a chunk's letter view.
 *
 * Each row is `[kind label mono] [surface] [chunk count]`. Clicking a row
 * activates the corresponding entity in the Navigator, filtering the
 * result list to chunks tagged with that entity.
 */
import { useT } from '../../lib/i18n/I18nContext';
import type { EntityRef } from '../../utils/tauriCommands';

interface MemoryChunkMentionedProps {
  entities: EntityRef[];
  onSelectEntity: (entity: EntityRef) => void;
}

export function MemoryChunkMentioned({ entities, onSelectEntity }: MemoryChunkMentionedProps) {
  const { t } = useT();
  if (entities.length === 0) return null;

  return (
    <section data-testid="memory-chunk-mentioned">
      <h3 className="mw-mentioned-heading">{t('intelligence.memoryChunk.mentioned.heading')}</h3>
      <div className="mw-mentioned-table">
        {entities.map(ent => (
          <button
            type="button"
            key={ent.entity_id}
            className="mw-mentioned-row"
            onClick={() => onSelectEntity(ent)}>
            <span className="mw-mentioned-kind">{ent.kind}</span>
            <span className="mw-mentioned-surface">{ent.surface}</span>
            <span className="mw-mentioned-count">
              {ent.count === 1
                ? t('intelligence.memoryChunk.mentioned.chunkOne')
                : t('intelligence.memoryChunk.mentioned.chunkOther').replace(
                    '{count}',
                    String(ent.count)
                  )}
            </span>
          </button>
        ))}
      </div>
    </section>
  );
}
