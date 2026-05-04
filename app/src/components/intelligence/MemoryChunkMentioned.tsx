/**
 * "Mentioned" entity list — the marginalia of a chunk's letter view.
 *
 * Each row is `[kind label mono] [surface] [chunk count]`. Clicking a row
 * activates the corresponding entity in the Navigator, filtering the
 * result list to chunks tagged with that entity.
 */
import type { EntityRef } from '../../utils/tauriCommands';

interface MemoryChunkMentionedProps {
  entities: EntityRef[];
  onSelectEntity: (entity: EntityRef) => void;
}

export function MemoryChunkMentioned({ entities, onSelectEntity }: MemoryChunkMentionedProps) {
  if (entities.length === 0) return null;

  return (
    <section data-testid="memory-chunk-mentioned">
      <h3 className="mw-mentioned-heading">m e n t i o n e d</h3>
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
              {ent.count} chunk{ent.count === 1 ? '' : 's'}
            </span>
          </button>
        ))}
      </div>
    </section>
  );
}
