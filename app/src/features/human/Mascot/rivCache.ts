// Version-keyed cache for backend-served Rive (.riv) binaries.
//
// The backend stamps each mascot with a `version`. We persist the downloaded
// binary in IndexedDB keyed by mascot id and only re-fetch from the backend
// when the stored version differs from the requested one — so switching back
// to a previously-seen mascot, or remounting the Human page, never re-hits the
// network. A small in-memory layer sits in front so repeated mounts in the
// same session skip even the IndexedDB round-trip.
//
// IndexedDB is best-effort: if it is unavailable (e.g. jsdom under Vitest, or a
// hardened webview) every helper degrades to a direct fetch with no caching.
import debug from 'debug';

const cacheLog = debug('human:mascot:riv-cache');

const DB_NAME = 'openhuman-mascots';
const STORE = 'riv';
const DB_VERSION = 1;

interface RivCacheEntry {
  id: string;
  version: string;
  buffer: ArrayBuffer;
  updatedAt: number;
}

/** Session-lifetime memory cache: id → { version, buffer }. */
const memCache = new Map<string, { version: string; buffer: ArrayBuffer }>();

function getIndexedDb(): IDBFactory | null {
  try {
    return typeof globalThis.indexedDB !== 'undefined' ? globalThis.indexedDB : null;
  } catch {
    return null;
  }
}

function openDb(): Promise<IDBDatabase | null> {
  const idb = getIndexedDb();
  if (!idb) return Promise.resolve(null);
  return new Promise(resolve => {
    let req: IDBOpenDBRequest;
    try {
      req = idb.open(DB_NAME, DB_VERSION);
    } catch (err) {
      cacheLog('indexedDB.open threw, skipping cache: %o', err);
      resolve(null);
      return;
    }
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains(STORE)) {
        db.createObjectStore(STORE, { keyPath: 'id' });
      }
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => {
      cacheLog('indexedDB.open error: %o', req.error);
      resolve(null);
    };
  });
}

function readEntry(db: IDBDatabase, id: string): Promise<RivCacheEntry | undefined> {
  return new Promise(resolve => {
    try {
      const tx = db.transaction(STORE, 'readonly');
      const req = tx.objectStore(STORE).get(id);
      req.onsuccess = () => resolve(req.result as RivCacheEntry | undefined);
      req.onerror = () => resolve(undefined);
    } catch {
      resolve(undefined);
    }
  });
}

function writeEntry(db: IDBDatabase, entry: RivCacheEntry): Promise<void> {
  return new Promise(resolve => {
    try {
      const tx = db.transaction(STORE, 'readwrite');
      tx.objectStore(STORE).put(entry);
      tx.oncomplete = () => resolve();
      tx.onerror = () => resolve();
      tx.onabort = () => resolve();
    } catch {
      resolve();
    }
  });
}

async function fetchBuffer(url: string): Promise<ArrayBuffer> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`failed to fetch riv (${res.status}) from ${url}`);
  return res.arrayBuffer();
}

/**
 * Resolve the .riv binary for a mascot, hitting the network only when the
 * cached version differs from `version`. Returns an ArrayBuffer suitable for
 * `useRive({ buffer })`.
 */
export async function loadRivBuffer(
  id: string,
  version: string,
  url: string
): Promise<ArrayBuffer> {
  const mem = memCache.get(id);
  if (mem && mem.version === version) {
    cacheLog('mem hit %s@%s', id, version);
    return mem.buffer;
  }

  const db = await openDb();
  if (db) {
    const entry = await readEntry(db, id);
    if (entry && entry.version === version) {
      cacheLog('idb hit %s@%s', id, version);
      memCache.set(id, { version, buffer: entry.buffer });
      db.close();
      return entry.buffer;
    }
  }

  cacheLog('miss %s@%s → fetching %s', id, version, url);
  const buffer = await fetchBuffer(url);
  memCache.set(id, { version, buffer });
  if (db) {
    await writeEntry(db, { id, version, buffer, updatedAt: Date.now() });
    db.close();
  }
  return buffer;
}

/** Test/maintenance helper — drops the in-memory layer. */
export function clearRivMemoryCache(): void {
  memCache.clear();
}
