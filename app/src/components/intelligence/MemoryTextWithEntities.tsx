/**
 * Renders memory query/recall text with highlighted entity type annotations,
 * plus an optional structured entity list when the backend returns entities
 * in the `context.entities[]` field.
 *
 * The backend surfaces entity types in text like:
 *   "Alice (PERSON) -[OWNS]-> Atlas (PROJECT)"
 *
 * This component parses those `(TYPE)` annotations and renders them as
 * small styled badges inline, keeping the rest as plain text.  When a
 * structured `entities` array is provided, it also renders a compact
 * entity chip bar above the text.
 */
import type { MemoryRetrievalEntity } from '../../utils/tauriCommands';

interface MemoryTextWithEntitiesProps {
  text: string;
  /** Structured entities from `context.entities[]` — shown as chips when present. */
  entities?: MemoryRetrievalEntity[];
  className?: string;
}

/** Matches parenthesized entity type annotations like (PERSON), (PROJECT), (ORG). */
const ENTITY_TYPE_RE = /\(([A-Z][A-Z0-9_]{1,30})\)/g;

/** Deterministic colour palette for entity type badges (hue-shifted). */
const TYPE_COLORS: Record<string, { bg: string; text: string; border: string }> = {
  PERSON: { bg: 'bg-sky-500/15', text: 'text-sky-300', border: 'border-sky-500/20' },
  PROJECT: { bg: 'bg-emerald-500/15', text: 'text-emerald-300', border: 'border-emerald-500/20' },
  ORG: { bg: 'bg-amber-500/15', text: 'text-amber-300', border: 'border-amber-500/20' },
  ORGANIZATION: { bg: 'bg-amber-500/15', text: 'text-amber-300', border: 'border-amber-500/20' },
  TECHNOLOGY: { bg: 'bg-violet-500/15', text: 'text-violet-300', border: 'border-violet-500/20' },
  TOOL: { bg: 'bg-violet-500/15', text: 'text-violet-300', border: 'border-violet-500/20' },
  LOCATION: { bg: 'bg-rose-500/15', text: 'text-rose-300', border: 'border-rose-500/20' },
  EVENT: { bg: 'bg-pink-500/15', text: 'text-pink-300', border: 'border-pink-500/20' },
  CONCEPT: { bg: 'bg-teal-500/15', text: 'text-teal-300', border: 'border-teal-500/20' },
};

const DEFAULT_TYPE_COLOR = {
  bg: 'bg-primary-500/15',
  text: 'text-primary-300',
  border: 'border-primary-500/20',
};

function colorForType(entityType: string): { bg: string; text: string; border: string } {
  return TYPE_COLORS[entityType.toUpperCase()] ?? DEFAULT_TYPE_COLOR;
}

interface TextSegment {
  kind: 'text' | 'entity-type';
  value: string;
}

function parseEntityAnnotations(text: string): TextSegment[] {
  const segments: TextSegment[] = [];
  let lastIndex = 0;

  for (const match of text.matchAll(ENTITY_TYPE_RE)) {
    const matchStart = match.index;
    if (matchStart > lastIndex) {
      segments.push({ kind: 'text', value: text.slice(lastIndex, matchStart) });
    }
    segments.push({ kind: 'entity-type', value: match[1] });
    lastIndex = matchStart + match[0].length;
  }

  if (lastIndex < text.length) {
    segments.push({ kind: 'text', value: text.slice(lastIndex) });
  }

  return segments;
}

/** Compact chip for a structured entity, showing name + optional type badge. */
function EntityChip({ entity }: { entity: MemoryRetrievalEntity }) {
  const color = entity.entity_type ? colorForType(entity.entity_type) : DEFAULT_TYPE_COLOR;
  return (
    <span
      className={`inline-flex items-center gap-1 px-1.5 py-0.5 rounded ${color.bg} border ${color.border}`}
      title={entity.entity_type ? `${entity.name} (${entity.entity_type})` : entity.name}>
      <span className={`text-[10px] leading-tight font-medium ${color.text}`}>{entity.name}</span>
      {entity.entity_type && (
        <span className="text-[8px] leading-tight font-semibold uppercase tracking-wide opacity-70">
          {entity.entity_type}
        </span>
      )}
    </span>
  );
}

export function MemoryTextWithEntities({ text, entities, className }: MemoryTextWithEntitiesProps) {
  if (!text && (!entities || entities.length === 0)) return null;

  const hasStructuredEntities = entities && entities.length > 0;
  const hasInlineAnnotations = ENTITY_TYPE_RE.test(text);
  // Reset regex lastIndex after the test above
  ENTITY_TYPE_RE.lastIndex = 0;

  return (
    <div className={className}>
      {/* Structured entity chips */}
      {hasStructuredEntities && (
        <div className="flex flex-wrap gap-1 mb-2 pb-2 border-b border-white/5">
          {entities.map((entity, i) => (
            <EntityChip key={entity.id ?? `${entity.name}-${i}`} entity={entity} />
          ))}
        </div>
      )}

      {/* Text content with inline entity type annotations */}
      {text && (
        <pre className="whitespace-pre-wrap m-0 p-0 font-inherit text-inherit leading-inherit">
          {hasInlineAnnotations
            ? parseEntityAnnotations(text).map((seg, i) =>
                seg.kind === 'entity-type' ? (
                  <span
                    key={i}
                    className={`inline-block mx-0.5 px-1 py-px rounded text-[9px] leading-tight font-semibold ${colorForType(seg.value).bg} ${colorForType(seg.value).text} border ${colorForType(seg.value).border} uppercase tracking-wide align-baseline`}
                    title={`Entity type: ${seg.value}`}>
                    {seg.value}
                  </span>
                ) : (
                  <span key={i}>{seg.value}</span>
                )
              )
            : text}
        </pre>
      )}
    </div>
  );
}
