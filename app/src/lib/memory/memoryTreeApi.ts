/**
 * memory_tree JSON-RPC client (with mock fixtures for development).
 *
 * Three-pane MemoryWorkspace browser uses this module as its only
 * data source. Real RPC calls land in a follow-up commit once the
 * sibling Rust worktree (feat/memory-cloud-default-backend) merges.
 *
 * Toggle `MEMORY_TREE_USE_MOCK` to `false` to switch from fixtures to
 * the real `openhuman.memory_tree_*` JSON-RPC methods.
 */

// ---------------------------------------------------------------------------
// Public types — match the memory_tree RPC contract
// ---------------------------------------------------------------------------

export type SourceKind = 'email' | 'chat' | 'screen' | 'voice' | 'doc';

export type LifecycleStatus = 'admitted' | 'buffered' | 'pending_extraction' | 'dropped';

export type EntityKind =
  | 'person'
  | 'organization'
  | 'location'
  | 'event'
  | 'product'
  | 'datetime'
  | 'technology'
  | 'artifact'
  | 'quantity'
  | 'misc';

/**
 * A single chunk in the memory tree — one user-visible message-sized unit
 * (an email, a chat turn, a doc page, a transcribed voice clip).
 */
export interface Chunk {
  id: string;
  source_kind: SourceKind;
  source_id: string;
  source_ref?: string;
  owner: string;
  timestamp_ms: number;
  token_count: number;
  lifecycle_status: LifecycleStatus;
  content_path?: string;
  /** Up to 500 chars; used as the result-list subject preview. */
  content_preview?: string;
  has_embedding: boolean;
  /** Hierarchical: ["person/Steve-Enamakel", "organization/TinyHumans"]. */
  tags: string[];
}

export interface ChunkFilter {
  source_kinds?: string[];
  source_ids?: string[];
  entity_ids?: string[];
  since_ms?: number;
  until_ms?: number;
  query?: string;
  limit?: number;
  offset?: number;
}

export interface Source {
  source_id: string;
  /** Un-slugged readable; user-email stripped. */
  display_name: string;
  source_kind: string;
  chunk_count: number;
  most_recent_ms: number;
  /** Aggregate lifecycle status of the most recent chunks (drives the dot color). */
  lifecycle_status: LifecycleStatus;
}

export interface EntityRef {
  /** "kind:surface" — e.g. "person:Steven Enamakel". */
  entity_id: string;
  kind: EntityKind;
  surface: string;
  count: number;
}

export interface ScoreSignal {
  name: string;
  weight: number;
  value: number;
}

export interface ScoreBreakdown {
  signals: ScoreSignal[];
  total: number;
  threshold: number;
  kept: boolean;
  llm_consulted: boolean;
}

// ---------------------------------------------------------------------------
// Toggle: mock vs real RPC
// ---------------------------------------------------------------------------

/**
 * When `true`, all memory_tree* calls return in-memory fixtures.
 * Flip to `false` once the JSON-RPC methods land in the Rust core.
 */
export const MEMORY_TREE_USE_MOCK = true;

// ---------------------------------------------------------------------------
// Mock fixtures
// ---------------------------------------------------------------------------

const NOW_MS = Date.UTC(2026, 4, 4, 9, 14, 0); // 2026-05-04 09:14 UTC — anchor for deterministic mocks
const HOUR = 60 * 60 * 1000;
const DAY = 24 * HOUR;

interface MockChunkSeed {
  id: string;
  source_kind: SourceKind;
  source_id: string;
  source_ref?: string;
  timestamp_ms: number;
  token_count: number;
  lifecycle_status?: LifecycleStatus;
  content_preview: string;
  tags: string[];
  has_embedding?: boolean;
}

const SEEDS: MockChunkSeed[] = [
  // ── today ──
  {
    id: 'chunk-354fa083',
    source_kind: 'email',
    source_id: 'gmail:enamakel@mail.tinyhumans.ai|sanil@vezures.xyz',
    source_ref: 'gmail://msg/19deb7a2b02d3a18',
    timestamp_ms: NOW_MS,
    token_count: 312,
    content_preview:
      "welcome to the future of ai assistants — openhuman. hey hey Sanil Jain! steve here. i'm super excited for you to try out openhuman! welcome to the family.",
    tags: ['person/Steven-Enamakel', 'organization/TinyHumans', 'product/openhuman'],
  },
  {
    id: 'chunk-1b7a92ee',
    source_kind: 'email',
    source_id: 'gmail:notifications@github.com|sanil@vezures.xyz',
    source_ref: 'gmail://msg/19deb7a2b02d3b29',
    timestamp_ms: NOW_MS - 90 * 60 * 1000,
    token_count: 94,
    content_preview:
      '[tinyhumansai/openhuman] PR #1175: feat(security): enforce prompt-injection guard before model and tool execution. merged.',
    tags: ['organization/GitHub', 'product/openhuman', 'event/pr-merged'],
  },
  {
    id: 'chunk-c4e0a1bb',
    source_kind: 'chat',
    source_id: 'slack:T0123|C-engineering',
    source_ref: 'slack://channel/C-engineering/p1714809000',
    timestamp_ms: NOW_MS - 3 * HOUR,
    token_count: 47,
    content_preview:
      'maya patel: i pushed the staging chart fix — local-llm fallback now respects the 8b cap',
    tags: ['person/Maya-Patel', 'organization/TinyHumans', 'technology/Helm'],
  },
  {
    id: 'chunk-7f128a30',
    source_kind: 'voice',
    source_id: 'otter:meeting-2026-05-04-design-sync',
    source_ref: 'otter://transcript/8XkP3LmQa',
    timestamp_ms: NOW_MS - 5 * HOUR,
    token_count: 1240,
    content_preview:
      'Steven Enamakel: ok — for the memory workspace rebuild, we want a three-pane browser, navigator on the left, result list in the middle, chunk detail on the right. think gmail meets a research notebook.',
    tags: [
      'person/Steven-Enamakel',
      'person/Maya-Patel',
      'product/openhuman',
      'technology/React',
      'event/design-sync',
    ],
  },
  // ── yesterday ──
  {
    id: 'chunk-9a2bc418',
    source_kind: 'email',
    source_id: 'gmail:no-reply@otter.ai|sanil@vezures.xyz',
    source_ref: 'gmail://msg/19deb7a2b02d3c4f',
    timestamp_ms: NOW_MS - 1 * DAY - 2 * HOUR,
    token_count: 78,
    content_preview:
      'your meeting "design sync — memory workspace" has been transcribed. 12 speakers detected. 47 minutes.',
    tags: ['organization/Otter.ai', 'event/transcription-ready'],
  },
  {
    id: 'chunk-eb71fa0c',
    source_kind: 'chat',
    source_id: 'slack:T0123|D-steve-enamakel',
    source_ref: 'slack://channel/D-steve-enamakel/p1714722000',
    timestamp_ms: NOW_MS - 1 * DAY - 4 * HOUR,
    token_count: 28,
    content_preview:
      'steven enamakel: have a look at the chunk_score signals doc once you wake up — i added "novelty" as a 4th dimension',
    tags: ['person/Steven-Enamakel', 'product/openhuman', 'technology/scoring'],
  },
  {
    id: 'chunk-2d4f8b91',
    source_kind: 'doc',
    source_id: 'gdrive:1AbCdEfGhIjKlMnOpQrSt',
    source_ref: 'gdrive://doc/1AbCdEfGhIjKlMnOpQrSt',
    timestamp_ms: NOW_MS - 1 * DAY - 8 * HOUR,
    token_count: 2200,
    content_preview:
      'memory_tree v2 design doc — chunks.db table layout, lifecycle states (admitted, buffered, pending_extraction, dropped), score breakdown, and the read-side JSON-RPC contract.',
    tags: ['organization/TinyHumans', 'product/openhuman', 'technology/SQLite', 'event/design-doc'],
    lifecycle_status: 'admitted',
  },
  {
    id: 'chunk-aa118801',
    source_kind: 'screen',
    source_id: 'screen-capture:vscode-2026-05-03',
    timestamp_ms: NOW_MS - 1 * DAY - 11 * HOUR,
    token_count: 156,
    content_preview:
      '[OCR from VS Code] src/openhuman/memory/tree/store.rs — added insert_chunk, list_chunks, score_chunk fns; tests pass locally.',
    tags: ['person/Sanil-Jain', 'technology/Rust', 'product/openhuman'],
    lifecycle_status: 'pending_extraction',
  },
  // ── this week ──
  ...buildWeekSeeds(),
  // ── older (2-3 weeks back) ──
  ...buildOlderSeeds(),
];

function buildWeekSeeds(): MockChunkSeed[] {
  // Wed 11:24, Tue 17:02, Mon 09:48 — three days back, varied sources
  return [
    {
      id: 'chunk-b2c3d4e5',
      source_kind: 'email',
      source_id: 'gmail:enamakel@mail.tinyhumans.ai|sanil@vezures.xyz',
      source_ref: 'gmail://msg/19deb7a2b02d3d51',
      timestamp_ms: NOW_MS - 2 * DAY - 1 * HOUR,
      token_count: 188,
      content_preview:
        "re: claude code 4.7 1m context — yes please. we should plan the worktree split for the workspace migration. i'll write up a plan tomorrow.",
      tags: [
        'person/Steven-Enamakel',
        'product/Claude-Code',
        'product/openhuman',
        'event/planning',
      ],
    },
    {
      id: 'chunk-c5d6e7f8',
      source_kind: 'chat',
      source_id: 'telegram:group-tinyhumans-eng',
      source_ref: 'tg://channel/-100123456789/m1234',
      timestamp_ms: NOW_MS - 2 * DAY - 6 * HOUR,
      token_count: 35,
      content_preview:
        'maya patel: pushing the memory-cloud-default-backend branch up — three rust worktrees in parallel, opus 4.7 driving',
      tags: ['person/Maya-Patel', 'product/openhuman', 'technology/Rust'],
    },
    {
      id: 'chunk-d7e8f9a0',
      source_kind: 'email',
      source_id: 'gmail:billing@anthropic.com|sanil@vezures.xyz',
      source_ref: 'gmail://msg/19deb7a2b02d3e62',
      timestamp_ms: NOW_MS - 3 * DAY - 4 * HOUR,
      token_count: 64,
      content_preview:
        'your anthropic api invoice for april 2026 — $1,247.18. usage breakdown attached. paid in full.',
      tags: ['organization/Anthropic', 'event/invoice'],
      lifecycle_status: 'admitted',
    },
    {
      id: 'chunk-e9f0a1b2',
      source_kind: 'voice',
      source_id: 'otter:meeting-2026-05-01-1on1-steve',
      source_ref: 'otter://transcript/9YoP4MnRb',
      timestamp_ms: NOW_MS - 3 * DAY - 7 * HOUR,
      token_count: 880,
      content_preview:
        'Steven Enamakel: ok so the memory workspace rebuild — three weeks of work, parallel worktrees, claude code does the bulk. i want it done by mid-may.',
      tags: ['person/Steven-Enamakel', 'person/Sanil-Jain', 'event/1on1', 'product/openhuman'],
    },
    {
      id: 'chunk-f1a2b3c4',
      source_kind: 'doc',
      source_id: 'github:tinyhumansai/openhuman/issues/1175',
      source_ref: 'github://issue/tinyhumansai/openhuman/1175',
      timestamp_ms: NOW_MS - 4 * DAY - 2 * HOUR,
      token_count: 410,
      content_preview:
        'enforce prompt-injection guard before model and tool execution. closes #1140. PR includes core unit tests + an e2e regression that asserts the guard sees system messages first.',
      tags: ['organization/GitHub', 'product/openhuman', 'event/security-fix'],
    },
    {
      id: 'chunk-a3b4c5d6',
      source_kind: 'chat',
      source_id: 'slack:T0123|C-design',
      source_ref: 'slack://channel/C-design/p1714463400',
      timestamp_ms: NOW_MS - 4 * DAY - 9 * HOUR,
      token_count: 22,
      content_preview:
        'maya patel: love the letterhead idea for the chunk detail pane — feels like reading a real letter.',
      tags: ['person/Maya-Patel', 'product/openhuman', 'event/design-feedback'],
    },
    {
      id: 'chunk-b5c6d7e8',
      source_kind: 'email',
      source_id: 'gmail:notifications@github.com|sanil@vezures.xyz',
      source_ref: 'gmail://msg/19deb7a2b02d3f73',
      timestamp_ms: NOW_MS - 5 * DAY - 3 * HOUR,
      token_count: 121,
      content_preview:
        '[tinyhumansai/openhuman] PR #1170 merged: fix(composio/gmail): phase out html2md, prefer text/plain MIME part',
      tags: ['organization/GitHub', 'product/openhuman', 'event/pr-merged'],
    },
    {
      id: 'chunk-c7d8e9f0',
      source_kind: 'screen',
      source_id: 'screen-capture:browser-2026-04-29',
      timestamp_ms: NOW_MS - 5 * DAY - 8 * HOUR,
      token_count: 220,
      content_preview:
        '[OCR from Chrome] reading react-window docs — virtualized list approach for large result sets, 60fps target on mid-range hardware.',
      tags: ['person/Sanil-Jain', 'technology/React', 'technology/react-window'],
      lifecycle_status: 'pending_extraction',
    },
    {
      id: 'chunk-d9e0f1a2',
      source_kind: 'chat',
      source_id: 'discord:guild-789|channel-general',
      source_ref: 'discord://channel/789/12345',
      timestamp_ms: NOW_MS - 6 * DAY - 1 * HOUR,
      token_count: 18,
      content_preview:
        'someone in #general: anyone tried the new openhuman beta? curious about the memory workspace.',
      tags: ['organization/Discord', 'product/openhuman', 'event/community-feedback'],
      lifecycle_status: 'buffered',
    },
  ];
}

function buildOlderSeeds(): MockChunkSeed[] {
  return [
    {
      id: 'chunk-old-1',
      source_kind: 'email',
      source_id: 'gmail:enamakel@mail.tinyhumans.ai|sanil@vezures.xyz',
      source_ref: 'gmail://msg/19deb7a2b02d4001',
      timestamp_ms: NOW_MS - 9 * DAY - 2 * HOUR,
      token_count: 256,
      content_preview:
        "project kickoff — memory_tree v2. attaching the architecture sketch + the interview notes from the user research round. let's sync wed.",
      tags: ['person/Steven-Enamakel', 'product/openhuman', 'event/kickoff'],
    },
    {
      id: 'chunk-old-2',
      source_kind: 'doc',
      source_id: 'gdrive:1ZyXwVuTsRqPoNmLkJiHg',
      source_ref: 'gdrive://doc/1ZyXwVuTsRqPoNmLkJiHg',
      timestamp_ms: NOW_MS - 11 * DAY - 6 * HOUR,
      token_count: 1850,
      content_preview:
        'user research synthesis — q1 2026. 14 interviews, 5 themes: memory longevity, recall precision, transparency of "why kept", source attribution, undo/redo.',
      tags: ['organization/TinyHumans', 'product/openhuman', 'event/research'],
    },
    {
      id: 'chunk-old-3',
      source_kind: 'chat',
      source_id: 'slack:T0123|C-engineering',
      source_ref: 'slack://channel/C-engineering/p1714032000',
      timestamp_ms: NOW_MS - 13 * DAY - 4 * HOUR,
      token_count: 33,
      content_preview:
        "sanil jain: dropping the old MemoryGraphMap — entity-entity edges aren't maintained in memory_tree, no point keeping it.",
      tags: ['person/Sanil-Jain', 'product/openhuman', 'event/cleanup'],
    },
    {
      id: 'chunk-old-4',
      source_kind: 'email',
      source_id: 'gmail:no-reply@otter.ai|sanil@vezures.xyz',
      source_ref: 'gmail://msg/19deb7a2b02d4112',
      timestamp_ms: NOW_MS - 15 * DAY - 8 * HOUR,
      token_count: 71,
      content_preview:
        'your weekly meeting summary — 7 meetings, 4h 32m total, 3 with action items pending.',
      tags: ['organization/Otter.ai', 'event/weekly-summary'],
      lifecycle_status: 'dropped',
    },
    {
      id: 'chunk-old-5',
      source_kind: 'voice',
      source_id: 'otter:meeting-2026-04-18-allhands',
      source_ref: 'otter://transcript/0ZpQ5NoSc',
      timestamp_ms: NOW_MS - 17 * DAY - 3 * HOUR,
      token_count: 3120,
      content_preview:
        'Steven Enamakel (all-hands, april): — q2 priorities. memory_tree v2, claude opus 4.7 1m context migration, e2e on linux + macos. shipping cadence stays weekly.',
      tags: [
        'person/Steven-Enamakel',
        'organization/TinyHumans',
        'event/all-hands',
        'product/openhuman',
      ],
    },
    {
      id: 'chunk-old-6',
      source_kind: 'screen',
      source_id: 'screen-capture:figma-2026-04-15',
      timestamp_ms: NOW_MS - 20 * DAY - 5 * HOUR,
      token_count: 88,
      content_preview:
        '[OCR from Figma] memory workspace — mood board. paper, hairlines, ocean accent, monochrome heatmap. inspiration: research notebooks, gmail, things 3.',
      tags: ['person/Maya-Patel', 'organization/TinyHumans', 'event/design-mood'],
      lifecycle_status: 'admitted',
    },
  ];
}

const MOCK_CHUNKS: Chunk[] = SEEDS.map(seed => ({
  id: seed.id,
  source_kind: seed.source_kind,
  source_id: seed.source_id,
  source_ref: seed.source_ref,
  owner: 'sanil@vezures.xyz',
  timestamp_ms: seed.timestamp_ms,
  token_count: seed.token_count,
  lifecycle_status: seed.lifecycle_status ?? 'admitted',
  content_preview: seed.content_preview,
  has_embedding: seed.has_embedding ?? true,
  tags: seed.tags,
}));

// Source display-name overrides for known IDs (un-slugged, user-email stripped)
const SOURCE_DISPLAY_NAMES: Record<string, string> = {
  'gmail:enamakel@mail.tinyhumans.ai|sanil@vezures.xyz': 'Steven Enamakel',
  'gmail:notifications@github.com|sanil@vezures.xyz': 'GitHub notifications',
  'gmail:no-reply@otter.ai|sanil@vezures.xyz': 'Otter.ai',
  'gmail:billing@anthropic.com|sanil@vezures.xyz': 'Anthropic billing',
  'slack:T0123|C-engineering': 'Slack: #engineering',
  'slack:T0123|C-design': 'Slack: #design',
  'slack:T0123|D-steve-enamakel': 'Slack: Steven Enamakel (DM)',
  'telegram:group-tinyhumans-eng': 'Telegram: tinyhumans-eng',
  'discord:guild-789|channel-general': 'Discord: #general',
  'otter:meeting-2026-05-04-design-sync': 'Otter: Design sync',
  'otter:meeting-2026-05-01-1on1-steve': 'Otter: 1:1 with Steven',
  'otter:meeting-2026-04-18-allhands': 'Otter: All-hands (April)',
  'gdrive:1AbCdEfGhIjKlMnOpQrSt': 'Drive: memory_tree v2 design',
  'gdrive:1ZyXwVuTsRqPoNmLkJiHg': 'Drive: User research synthesis',
  'github:tinyhumansai/openhuman/issues/1175': 'GitHub: openhuman #1175',
  'screen-capture:vscode-2026-05-03': 'Screen: VS Code',
  'screen-capture:browser-2026-04-29': 'Screen: Chrome',
  'screen-capture:figma-2026-04-15': 'Screen: Figma',
};

function aggregateLifecycle(chunks: Chunk[]): LifecycleStatus {
  // Most recent N — drives the dot color
  const recent = chunks.slice(0, 5);
  if (recent.some(c => c.lifecycle_status === 'dropped')) return 'dropped';
  if (recent.some(c => c.lifecycle_status === 'pending_extraction')) return 'pending_extraction';
  if (recent.some(c => c.lifecycle_status === 'buffered')) return 'buffered';
  return 'admitted';
}

function deriveDisplayName(sourceId: string, kind: string): string {
  if (SOURCE_DISPLAY_NAMES[sourceId]) return SOURCE_DISPLAY_NAMES[sourceId];
  // Fallback: strip the bar-separated user email suffix and unslug the rest
  const rawTail = sourceId.split('|')[0];
  const afterColon = rawTail.includes(':') ? rawTail.split(':').slice(1).join(':') : rawTail;
  const cleaned = afterColon.replace(/-/g, ' ').trim();
  if (!cleaned) return `${kind}: ${sourceId}`;
  return cleaned;
}

// ---------------------------------------------------------------------------
// Mock implementations (sync-shaped via Promise.resolve)
// ---------------------------------------------------------------------------

function chunkMatchesFilter(chunk: Chunk, filter: ChunkFilter): boolean {
  if (filter.source_kinds && filter.source_kinds.length > 0) {
    if (!filter.source_kinds.includes(chunk.source_kind)) return false;
  }
  if (filter.source_ids && filter.source_ids.length > 0) {
    if (!filter.source_ids.includes(chunk.source_id)) return false;
  }
  if (filter.entity_ids && filter.entity_ids.length > 0) {
    const hit = filter.entity_ids.some(eid => chunk.tags.includes(eid));
    if (!hit) return false;
  }
  if (typeof filter.since_ms === 'number' && chunk.timestamp_ms < filter.since_ms) return false;
  if (typeof filter.until_ms === 'number' && chunk.timestamp_ms > filter.until_ms) return false;
  if (filter.query) {
    const needle = filter.query.toLowerCase();
    const hay = `${chunk.content_preview ?? ''} ${chunk.tags.join(' ')}`.toLowerCase();
    if (!hay.includes(needle)) return false;
  }
  return true;
}

async function mockListChunks(filter: ChunkFilter): Promise<{ chunks: Chunk[]; total: number }> {
  const all = [...MOCK_CHUNKS]
    .filter(c => chunkMatchesFilter(c, filter))
    .sort((a, b) => b.timestamp_ms - a.timestamp_ms);
  const total = all.length;
  const offset = filter.offset ?? 0;
  const limit = filter.limit ?? all.length;
  const chunks = all.slice(offset, offset + limit);
  return Promise.resolve({ chunks, total });
}

async function mockListSources(): Promise<Source[]> {
  const bySource = new Map<string, Chunk[]>();
  for (const chunk of MOCK_CHUNKS) {
    const list = bySource.get(chunk.source_id) ?? [];
    list.push(chunk);
    bySource.set(chunk.source_id, list);
  }
  const sources: Source[] = [];
  for (const [sourceId, chunks] of bySource) {
    chunks.sort((a, b) => b.timestamp_ms - a.timestamp_ms);
    sources.push({
      source_id: sourceId,
      display_name: deriveDisplayName(sourceId, chunks[0].source_kind),
      source_kind: chunks[0].source_kind,
      chunk_count: chunks.length,
      most_recent_ms: chunks[0].timestamp_ms,
      lifecycle_status: aggregateLifecycle(chunks),
    });
  }
  sources.sort((a, b) => b.most_recent_ms - a.most_recent_ms);
  return Promise.resolve(sources);
}

async function mockSearch(query: string, k: number): Promise<Chunk[]> {
  const result = await mockListChunks({ query, limit: k });
  return result.chunks;
}

async function mockRecall(
  query: string,
  k: number
): Promise<{ chunks: Chunk[]; scores: number[] }> {
  const result = await mockListChunks({ query, limit: k });
  // Mock scores: deterministic descending from 0.92 down by 0.07
  const scores = result.chunks.map((_, i) => Math.max(0.05, 0.92 - i * 0.07));
  return { chunks: result.chunks, scores };
}

function tagToEntityRef(tag: string, count: number): EntityRef | null {
  const slashIdx = tag.indexOf('/');
  if (slashIdx <= 0) return null;
  const kindRaw = tag.slice(0, slashIdx);
  const surfaceRaw = tag.slice(slashIdx + 1);
  const kindSet: Record<string, EntityKind> = {
    person: 'person',
    organization: 'organization',
    location: 'location',
    event: 'event',
    product: 'product',
    datetime: 'datetime',
    technology: 'technology',
    artifact: 'artifact',
    quantity: 'quantity',
    misc: 'misc',
  };
  const kind = kindSet[kindRaw] ?? 'misc';
  const surface = surfaceRaw.replace(/-/g, ' ');
  return { entity_id: `${kind}:${surface}`, kind, surface, count };
}

async function mockEntityIndexFor(chunkId: string): Promise<EntityRef[]> {
  const chunk = MOCK_CHUNKS.find(c => c.id === chunkId);
  if (!chunk) return Promise.resolve([]);
  // Each tag → one EntityRef; counts simulated as how many other chunks share that tag
  const counts = new Map<string, number>();
  for (const c of MOCK_CHUNKS) {
    for (const tag of c.tags) {
      counts.set(tag, (counts.get(tag) ?? 0) + 1);
    }
  }
  const refs: EntityRef[] = [];
  for (const tag of chunk.tags) {
    const ref = tagToEntityRef(tag, counts.get(tag) ?? 1);
    if (ref) refs.push(ref);
  }
  return Promise.resolve(refs);
}

async function mockTopEntities(kind?: string, limit?: number): Promise<EntityRef[]> {
  const counts = new Map<string, number>();
  for (const c of MOCK_CHUNKS) {
    for (const tag of c.tags) {
      counts.set(tag, (counts.get(tag) ?? 0) + 1);
    }
  }
  const refs: EntityRef[] = [];
  for (const [tag, count] of counts) {
    const ref = tagToEntityRef(tag, count);
    if (!ref) continue;
    if (kind && ref.kind !== kind) continue;
    refs.push(ref);
  }
  refs.sort((a, b) => b.count - a.count);
  return Promise.resolve(typeof limit === 'number' ? refs.slice(0, limit) : refs);
}

async function mockChunkScore(chunkId: string): Promise<ScoreBreakdown> {
  // Deterministic score breakdown derived from the chunk id hash.
  const chunk = MOCK_CHUNKS.find(c => c.id === chunkId);
  if (!chunk) {
    return Promise.resolve({
      signals: [],
      total: 0,
      threshold: 0.85,
      kept: false,
      llm_consulted: false,
    });
  }
  let h = 0;
  for (let i = 0; i < chunkId.length; i++) h = (h * 31 + chunkId.charCodeAt(i)) | 0;
  const seed = (Math.abs(h) % 1000) / 1000;
  const sourceVal = +(0.4 + ((seed * 17) % 1) * 0.5).toFixed(2);
  const entitiesVal = +(0.5 + ((seed * 31) % 1) * 0.45).toFixed(2);
  const recencyVal =
    chunk.timestamp_ms > NOW_MS - 7 * DAY
      ? +(0.78 + ((seed * 11) % 1) * 0.2).toFixed(2)
      : +(0.3 + ((seed * 7) % 1) * 0.4).toFixed(2);
  const signals: ScoreSignal[] = [
    { name: 'source', weight: 0.3, value: sourceVal },
    { name: 'entities', weight: 0.4, value: entitiesVal },
    { name: 'recency', weight: 0.3, value: recencyVal },
  ];
  const total = +signals.reduce((sum, s) => sum + s.weight * s.value, 0).toFixed(2);
  const threshold = 0.85;
  return Promise.resolve({
    signals,
    total,
    threshold,
    kept: chunk.lifecycle_status === 'admitted',
    llm_consulted: total >= threshold - 0.1 && total < threshold + 0.05,
  });
}

async function mockDeleteChunk(_chunkId: string): Promise<void> {
  // Mock layer is read-mostly; deletion is a no-op for fixtures.
  return Promise.resolve();
}

// ---------------------------------------------------------------------------
// Real RPC stubs (TODO: wire when Worktree 1 lands)
// ---------------------------------------------------------------------------

function notWired(method: string): never {
  throw new Error(
    `[memoryTreeApi] ${method} not yet wired — flip MEMORY_TREE_USE_MOCK to false only after the Rust core ships memory_tree_* RPCs.`
  );
}

async function tauriListChunks(_filter: ChunkFilter): Promise<{ chunks: Chunk[]; total: number }> {
  notWired('listChunks');
}
async function tauriListSources(): Promise<Source[]> {
  notWired('listSources');
}
async function tauriSearch(_query: string, _k: number): Promise<Chunk[]> {
  notWired('search');
}
async function tauriRecall(
  _query: string,
  _k: number
): Promise<{ chunks: Chunk[]; scores: number[] }> {
  notWired('recall');
}
async function tauriEntityIndexFor(_chunkId: string): Promise<EntityRef[]> {
  notWired('entityIndexFor');
}
async function tauriTopEntities(_kind?: string, _limit?: number): Promise<EntityRef[]> {
  notWired('topEntities');
}
async function tauriChunkScore(_chunkId: string): Promise<ScoreBreakdown> {
  notWired('chunkScore');
}
async function tauriDeleteChunk(_chunkId: string): Promise<void> {
  notWired('deleteChunk');
}

// ---------------------------------------------------------------------------
// Public API surface
// ---------------------------------------------------------------------------

export const memoryTreeApi = {
  listChunks: (filter: ChunkFilter) =>
    MEMORY_TREE_USE_MOCK ? mockListChunks(filter) : tauriListChunks(filter),
  listSources: () => (MEMORY_TREE_USE_MOCK ? mockListSources() : tauriListSources()),
  search: (query: string, k: number) =>
    MEMORY_TREE_USE_MOCK ? mockSearch(query, k) : tauriSearch(query, k),
  recall: (query: string, k: number) =>
    MEMORY_TREE_USE_MOCK ? mockRecall(query, k) : tauriRecall(query, k),
  entityIndexFor: (chunkId: string) =>
    MEMORY_TREE_USE_MOCK ? mockEntityIndexFor(chunkId) : tauriEntityIndexFor(chunkId),
  topEntities: (kind?: string, limit?: number) =>
    MEMORY_TREE_USE_MOCK ? mockTopEntities(kind, limit) : tauriTopEntities(kind, limit),
  chunkScore: (chunkId: string) =>
    MEMORY_TREE_USE_MOCK ? mockChunkScore(chunkId) : tauriChunkScore(chunkId),
  deleteChunk: (chunkId: string) =>
    MEMORY_TREE_USE_MOCK ? mockDeleteChunk(chunkId) : tauriDeleteChunk(chunkId),
};

// ---------------------------------------------------------------------------
// Test-only helpers (not exported via barrel — referenced directly in unit tests)
// ---------------------------------------------------------------------------

/** @internal — exposed for unit tests. */
export const __MOCK_CHUNKS__ = MOCK_CHUNKS;
