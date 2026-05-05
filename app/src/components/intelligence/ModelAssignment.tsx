import {
  DEFAULT_EXTRACT_MODEL,
  DEFAULT_SUMMARISER_MODEL,
  type ModelDescriptor,
  RECOMMENDED_MODEL_CATALOG,
  REQUIRED_EMBEDDER_MODEL,
} from '../../lib/intelligence/settingsApi';

interface ModelAssignmentProps {
  /** Names of models that are already installed on the user's machine. */
  installedModelIds: ReadonlyArray<string>;
  /** Currently chosen memory LLM (used for both extract + summarise). */
  memoryModel: string;
  /** Called when the user picks a different memory LLM. The setting fans
   *  out to both `llm_extractor_model` and `llm_summariser_model` in
   *  config.toml — most users want one model for both roles, and the
   *  cognitive load of two dropdowns isn't worth the rare power-user
   *  case of mixing them. */
  onChangeMemory: (id: string) => void;
}

/**
 * Per-role assignment table — two rows: Memory LLM (covers both extract
 * and summarise), and Embedder.
 *
 * The embedder row is locked to `bge-m3` for v1 (the spec says we never
 * round-trip embeddings through the cloud). The Memory LLM dropdown is
 * populated from the recommended catalog filtered to models that can
 * serve both extract AND summarise roles, plus any locally-installed
 * models the user has pulled outside the curated catalog.
 */
export default function ModelAssignment({
  installedModelIds,
  memoryModel,
  onChangeMemory,
}: ModelAssignmentProps) {
  // Ollama returns tags as `<name>:latest` for default-tag models. The
  // catalog stores bare names (e.g. `bge-m3`). Strip the `:latest` suffix
  // on the installed side so the bare-name comparison matches.
  const normalizedInstalled = installedModelIds.map(id =>
    id.endsWith(':latest') ? id.slice(0, -':latest'.length) : id
  );
  const memoryOptions = memoryLlmOptions(normalizedInstalled);
  const embedderDescriptor = RECOMMENDED_MODEL_CATALOG.find(m => m.id === REQUIRED_EMBEDDER_MODEL);
  const embedderInstalled = normalizedInstalled.includes(REQUIRED_EMBEDDER_MODEL);

  return (
    <div className="border border-stone-200 rounded-2xl overflow-hidden">
      <Row
        label="Memory LLM"
        sublabel={describeMemory(memoryOptions.find(opt => opt.id === memoryModel))}>
        <select
          value={memoryModel}
          onChange={e => onChangeMemory(e.target.value)}
          className="w-full sm:w-64 px-3 py-1.5 text-sm bg-white border border-stone-200 rounded-lg text-stone-900 focus:outline-none focus:border-primary-500/50 transition-colors"
          aria-label="Memory LLM (extract + summarise)">
          {memoryOptions.map(opt => (
            <option key={opt.id} value={opt.id}>
              {opt.label ?? opt.id}
            </option>
          ))}
        </select>
      </Row>

      <Row
        label="Embedder"
        sublabel={
          embedderDescriptor
            ? `${embedderDescriptor.size} · required · 1024-dim`
            : 'required · 1024-dim'
        }
        last>
        <div className="flex items-center gap-2 text-sm font-mono text-stone-700">
          <span>{REQUIRED_EMBEDDER_MODEL}</span>
          {embedderInstalled ? (
            <span className="inline-flex items-center gap-1 text-sage-600 text-xs">
              <svg
                className="w-3 h-3"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth={2.5}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
              </svg>
              loaded
            </span>
          ) : (
            <span className="text-amber-700 text-xs">not downloaded</span>
          )}
        </div>
      </Row>
    </div>
  );
}

interface RowProps {
  label: string;
  sublabel: string;
  last?: boolean;
  children: React.ReactNode;
}

function Row({ label, sublabel, last, children }: RowProps) {
  return (
    <div
      className={`grid grid-cols-1 sm:grid-cols-[1fr_auto] gap-2 sm:gap-6 px-5 py-4 ${
        last ? '' : 'border-b border-stone-100'
      }`}>
      <div>
        <div className="text-sm font-semibold text-stone-900">{label}</div>
        <div className="font-mono text-[11px] text-stone-500 mt-0.5">{sublabel}</div>
      </div>
      <div className="flex items-center sm:justify-end">{children}</div>
    </div>
  );
}

function describeMemory(model?: ModelDescriptor): string {
  if (!model) return 'used for extract + summarise';
  return `${model.size} · ${model.ramHint} · ${model.category}`;
}

/**
 * Build the Memory LLM dropdown options. A model qualifies if it can serve
 * BOTH extract and summarise roles. Catalog entries come first; locally
 * installed extras (pulled outside the curated catalog) are appended so
 * they remain selectable.
 */
function memoryLlmOptions(installedModelIds: ReadonlyArray<string>): ModelDescriptor[] {
  const catalog = RECOMMENDED_MODEL_CATALOG.filter(
    m => m.roles.includes('extract') && m.roles.includes('summariser')
  );
  const known = new Set(catalog.map(m => m.id));
  const extras = installedModelIds
    .filter(id => !known.has(id) && id !== REQUIRED_EMBEDDER_MODEL)
    .map<ModelDescriptor>(id => ({
      id,
      size: '—',
      approxBytes: 0,
      ramHint: '—',
      category: 'balanced',
      note: 'locally installed',
      roles: ['extract', 'summariser'],
    }));
  return [...catalog, ...extras];
}

// Re-export defaults so callers can still seed initial state via these
// constants without chasing them through the API module.
export { DEFAULT_EXTRACT_MODEL, DEFAULT_SUMMARISER_MODEL, REQUIRED_EMBEDDER_MODEL };
