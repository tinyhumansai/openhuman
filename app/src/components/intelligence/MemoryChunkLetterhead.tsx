/**
 * Letterhead: the from / to / date frontmatter of a chunk, rendered
 * as correspondence (dl-style with monospace labels in a fixed column).
 */
import type { Chunk } from '../../utils/tauriCommands';

interface LetterheadParts {
  fromName: string;
  fromAddress?: string;
  toAddress: string;
}

function parseSourceParts(chunk: Chunk): LetterheadParts {
  const left = chunk.source_id.split('|');
  const senderRaw = left[0];
  const recipient = left[1] ?? chunk.owner;
  const afterColon = senderRaw.includes(':') ? senderRaw.split(':').slice(1).join(':') : senderRaw;

  // Heuristic for known prefixes: prefer the human-readable display when we have one,
  // else fall back to the raw email/handle.
  const isEmailish = /@/.test(afterColon);
  // Try to recover a personalized name from the chunk's tags (first person/* tag)
  const personTag = chunk.tags.find(t => t.startsWith('person/'));
  const personName = personTag ? personTag.slice('person/'.length).replace(/-/g, ' ') : null;

  if (isEmailish && personName) {
    return { fromName: personName, fromAddress: afterColon, toAddress: recipient };
  }
  if (isEmailish) {
    return { fromName: afterColon, toAddress: recipient };
  }
  if (personName) {
    return { fromName: personName, fromAddress: afterColon, toAddress: recipient };
  }
  return { fromName: afterColon || chunk.source_kind, toAddress: recipient };
}

function formatLetterDate(ms: number): string {
  const d = new Date(ms);
  const yyyy = d.getUTCFullYear();
  const mm = String(d.getUTCMonth() + 1).padStart(2, '0');
  const dd = String(d.getUTCDate()).padStart(2, '0');
  const hh = String(d.getUTCHours()).padStart(2, '0');
  const mi = String(d.getUTCMinutes()).padStart(2, '0');
  return `${yyyy}·${mm}·${dd} · ${hh}:${mi} utc`;
}

export function MemoryChunkLetterhead({ chunk }: { chunk: Chunk }) {
  const parts = parseSourceParts(chunk);
  return (
    <header className="mw-letterhead" data-testid="memory-chunk-letterhead">
      <dl style={{ margin: 0 }}>
        <div className="mw-letterhead-row">
          <dt className="mw-letterhead-label">from</dt>
          <dd className="mw-letterhead-value" style={{ margin: 0 }}>
            {parts.fromName}
            {parts.fromAddress && parts.fromAddress !== parts.fromName && (
              <span className="mw-letterhead-value-secondary">{parts.fromAddress}</span>
            )}
          </dd>
        </div>
        <div className="mw-letterhead-row">
          <dt className="mw-letterhead-label">to</dt>
          <dd className="mw-letterhead-value" style={{ margin: 0 }}>
            {parts.toAddress}
          </dd>
        </div>
      </dl>
      <div className="mw-letterhead-date">{formatLetterDate(chunk.timestamp_ms)}</div>
    </header>
  );
}
