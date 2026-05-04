/**
 * Right pane — single-chunk detail rendered as correspondence (a letter):
 *
 *   1. Letterhead     (from / to / date)
 *   2. Subject + body (markdown-ish prose; entities highlighted)
 *   3. Mentioned      (entity index for the chunk)
 *   4. Why kept       (signal breakdown + threshold)
 *   5. Footer         (source_ref, chunk id, embedder info)
 */
import { useEffect, useState } from 'react';

import {
  type Chunk,
  type EntityRef,
  memoryTreeChunkScore,
  memoryTreeEntityIndexFor,
  type ScoreBreakdown,
} from '../../utils/tauriCommands';
import { MemoryChunkLetterhead } from './MemoryChunkLetterhead';
import { MemoryChunkMentioned } from './MemoryChunkMentioned';
import { MemoryChunkScoreBars } from './MemoryChunkScoreBars';
import { MemoryTextWithEntities } from './MemoryTextWithEntities';

interface MemoryChunkDetailProps {
  chunk: Chunk;
  onSelectEntity: (entity: EntityRef) => void;
}

function deriveSubject(chunk: Chunk): string {
  const preview = (chunk.content_preview ?? '').trim();
  if (!preview) return chunk.id;
  const firstLine = preview.split(/[.!?\n]/)[0]?.trim() ?? '';
  if (firstLine.length === 0) return chunk.id;
  return firstLine.length > 200 ? `${firstLine.slice(0, 197)}…` : firstLine;
}

function deriveBody(chunk: Chunk): string {
  const preview = (chunk.content_preview ?? '').trim();
  if (!preview) return '';
  // Drop the subject (first sentence/line) when there's more content after it.
  const firstBreak = preview.search(/[.!?\n](?:\s|$)/);
  if (firstBreak > 0 && preview.length > firstBreak + 2) {
    return preview.slice(firstBreak + 1).trim();
  }
  return preview;
}

function shortChunkId(id: string): string {
  // Trim "chunk-" prefix if present, then take 8 chars.
  const stripped = id.startsWith('chunk-') ? id.slice('chunk-'.length) : id;
  return stripped.slice(0, 8);
}

export function MemoryChunkDetail({ chunk, onSelectEntity }: MemoryChunkDetailProps) {
  const [entities, setEntities] = useState<EntityRef[]>([]);
  const [breakdown, setBreakdown] = useState<ScoreBreakdown | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let cancelled = false;
    console.debug('[ui-flow][memory-workspace] loading detail for chunk', chunk.id);
    void Promise.all([memoryTreeEntityIndexFor(chunk.id), memoryTreeChunkScore(chunk.id)]).then(
      ([ents, score]) => {
        if (cancelled) return;
        setEntities(ents);
        setBreakdown(score);
        console.debug(
          '[ui-flow][memory-workspace] detail loaded',
          chunk.id,
          'entities=',
          ents.length,
          'score_total=',
          score?.total ?? null
        );
      }
    );
    return () => {
      cancelled = true;
    };
  }, [chunk.id]);

  const handleCopyId = async () => {
    try {
      if (typeof navigator !== 'undefined' && navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(chunk.id);
      }
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch (err) {
      console.warn('[ui-flow][memory-workspace] copy chunk id failed', err);
    }
  };

  const subject = deriveSubject(chunk);
  const body = deriveBody(chunk);

  return (
    <article className="mw-pane-detail" data-testid="memory-chunk-detail">
      <div className="mw-pane-scroll mw-detail-scroll">
        <div className="mw-letter">
          <MemoryChunkLetterhead chunk={chunk} />

          <hr className="mw-rule" />

          <h1 className="mw-letter-subject">{subject}</h1>
          {body && (
            <div className="mw-letter-body">
              <MemoryTextWithEntities text={body} />
            </div>
          )}

          {entities.length > 0 && <hr className="mw-rule" />}

          <MemoryChunkMentioned entities={entities} onSelectEntity={onSelectEntity} />

          {breakdown && <hr className="mw-rule" />}

          {breakdown && <MemoryChunkScoreBars breakdown={breakdown} />}

          <footer className="mw-letter-footer">
            {chunk.source_ref && <span>{chunk.source_ref}</span>}
            <span>·</span>
            <button type="button" onClick={() => void handleCopyId()} title="Copy chunk id">
              chunk {shortChunkId(chunk.id)}
              {copied && <span style={{ marginLeft: 6, color: 'var(--sage)' }}>copied</span>}
            </button>
            <span>·</span>
            <span>{chunk.has_embedding ? 'bge-m3 1024dim' : 'no embedding'}</span>
          </footer>
        </div>
      </div>
    </article>
  );
}
