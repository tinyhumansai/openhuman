/**
 * In-process replacement for the removed `openhuman::ai_memory` core RPC surface.
 * Keeps session + memory index behavior in RAM for the desktop UI (no disk persistence).
 */
import type { ChunkRecordRust } from './memory/types';
import type { SessionEntry } from './sessions/types';

interface FileRecordJson {
  path: string;
  source: string;
  hash: string;
  mtime: number;
  size: number;
}

const memoryFiles = new Map<string, string>();
const fileRecords = new Map<string, FileRecordJson>();
const chunks = new Map<string, ChunkRecordRust>();
const chunksByPath = new Map<string, Set<string>>();
const metaKv = new Map<string, string>();
const embeddingCache = new Map<
  string,
  {
    provider: string;
    model: string;
    hash: string;
    embedding: number[];
    dims: number | null;
    updated_at: number;
  }
>();

let sessionIndex: Record<string, SessionEntry> = {};
const transcripts = new Map<string, string[]>();

function cacheKey(provider: string, model: string, hash: string): string {
  return `${provider}\0${model}\0${hash}`;
}

function ftsScore(text: string, query: string): number {
  const q = query.trim().toLowerCase();
  if (!q) return 0;
  const t = text.toLowerCase();
  let score = 0;
  for (const word of q.split(/\s+/)) {
    if (word && t.includes(word)) score += 1;
  }
  return score;
}

export async function dispatchLocalAiMethod(
  method: string,
  params: Record<string, unknown>
): Promise<unknown> {
  switch (method) {
    case 'ai.list_memory_files': {
      const dir = (params.relative_dir as string | undefined) ?? 'memory';
      const prefix = dir.endsWith('/') ? dir : `${dir}/`;
      const names: string[] = [];
      for (const k of memoryFiles.keys()) {
        if (k === dir || k.startsWith(prefix)) {
          const rest = k.startsWith(prefix) ? k.slice(prefix.length) : k;
          if (rest && !rest.includes('/')) names.push(rest);
        }
      }
      return names;
    }
    case 'ai.read_memory_file': {
      const path = params.relative_path as string;
      const v = memoryFiles.get(path);
      if (v === undefined) throw new Error(`memory file not found: ${path}`);
      return v;
    }
    case 'ai.write_memory_file': {
      const path = params.relative_path as string;
      const content = params.content as string;
      memoryFiles.set(path, content);
      return true;
    }
    case 'ai.memory_init':
      return true;
    case 'ai.memory_get_file': {
      const path = params.path as string;
      return fileRecords.get(path) ?? null;
    }
    case 'ai.memory_delete_chunks_by_path': {
      const path = params.path as string;
      const ids = chunksByPath.get(path);
      if (!ids) return 0;
      let n = 0;
      for (const id of ids) {
        chunks.delete(id);
        n++;
      }
      chunksByPath.delete(path);
      return n;
    }
    case 'ai.memory_upsert_chunk': {
      const chunk = params.chunk as ChunkRecordRust;
      chunks.set(chunk.id, chunk);
      let set = chunksByPath.get(chunk.path);
      if (!set) {
        set = new Set();
        chunksByPath.set(chunk.path, set);
      }
      set.add(chunk.id);
      return true;
    }
    case 'ai.memory_upsert_file': {
      const file = params.file as FileRecordJson;
      fileRecords.set(file.path, file);
      return true;
    }
    case 'ai.memory_set_meta': {
      metaKv.set(params.key as string, params.value as string);
      return true;
    }
    case 'ai.memory_get_meta':
      return metaKv.get(params.key as string) ?? null;
    case 'ai.memory_fts_search': {
      const query = params.query as string;
      const limit = Number(params.limit ?? 20);
      const out: Array<{
        chunk_id: string;
        path: string;
        source: string;
        text: string;
        score: number;
        start_line: number;
        end_line: number;
        updated_at?: number;
      }> = [];
      for (const ch of chunks.values()) {
        const sc = ftsScore(ch.text, query);
        if (sc <= 0) continue;
        out.push({
          chunk_id: ch.id,
          path: ch.path,
          source: ch.source,
          text: ch.text,
          score: sc,
          start_line: ch.start_line,
          end_line: ch.end_line,
          updated_at: ch.updated_at,
        });
      }
      out.sort((a, b) => b.score - a.score);
      return out.slice(0, limit);
    }
    case 'ai.memory_get_all_embeddings': {
      const rows: [string, number[]][] = [];
      for (const ch of chunks.values()) {
        if (ch.embedding && ch.embedding.length) rows.push([ch.id, ch.embedding]);
      }
      return rows;
    }
    case 'ai.memory_get_chunks': {
      const path = params.path as string;
      const ids = chunksByPath.get(path);
      if (!ids) return [];
      return [...ids].map(id => chunks.get(id)).filter(Boolean);
    }
    case 'ai.memory_cache_embedding': {
      const entry = params.entry as {
        provider: string;
        model: string;
        hash: string;
        embedding: number[];
        dims: number | null;
        updatedAt: number;
      };
      embeddingCache.set(cacheKey(entry.provider, entry.model, entry.hash), {
        provider: entry.provider,
        model: entry.model,
        hash: entry.hash,
        embedding: entry.embedding,
        dims: entry.dims,
        updated_at: entry.updatedAt,
      });
      return true;
    }
    case 'ai.memory_get_cached_embedding': {
      const provider = params.provider as string;
      const model = params.model as string;
      const hash = params.hash as string;
      const row = embeddingCache.get(cacheKey(provider, model, hash));
      return row?.embedding ?? null;
    }
    case 'ai.sessions_init':
      return true;
    case 'ai.sessions_load_index':
      return { ...sessionIndex };
    case 'ai.sessions_update_index': {
      const session_id = params.session_id as string;
      const entry = params.entry as SessionEntry;
      sessionIndex[session_id] = { ...entry };
      return true;
    }
    case 'ai.sessions_append_transcript': {
      const session_id = params.session_id as string;
      const line = params.line as string;
      const prev = transcripts.get(session_id) ?? [];
      prev.push(line);
      transcripts.set(session_id, prev);
      return true;
    }
    case 'ai.sessions_read_transcript':
      return transcripts.get(params.session_id as string) ?? [];
    case 'ai.sessions_delete': {
      const session_id = params.session_id as string;
      delete sessionIndex[session_id];
      transcripts.delete(session_id);
      return true;
    }
    case 'ai.sessions_list':
      return Object.keys(sessionIndex);
    default:
      throw new Error(`local AI dispatch: unknown method ${method}`);
  }
}
