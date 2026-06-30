import debug from 'debug';

import { callCoreRpc } from '../coreRpcClient';

const log = debug('skillRegistryApi');

/**
 * Catalog reads (`browse`/`search`/`sources`/`categories`) all funnel through
 * the backend's single-flight `browse_catalog`, whose COLD fetch downloads the
 * full ~90k-entry registry and can take ~80s. That comfortably exceeds the
 * default 30s `CORE_RPC_TIMEOUT_MS`, so a first load (or post-TTL revalidate)
 * would spuriously time out. Give these the longer per-call timeout the RPC
 * client supports for exactly such "slow-but-alive" calls; warm-cache reads
 * still return in milliseconds, so this only raises the ceiling for the rare
 * cold path. 120s = ~80s cold download + margin (well under the 10min clamp).
 */
const CATALOG_RPC_TIMEOUT_MS = 120_000;

export interface CatalogEntry {
  id: string;
  name: string;
  description: string;
  source: string;
  category: string;
  author: string | null;
  version: string | null;
  tags: string[];
  platforms: string[];
  download_url: string;
  docs_path: string | null;
  commands: string[];
  env_vars: string[];
  license: string | null;
}

export interface RegistryInstallResult {
  url: string;
  stdout: string;
  stderr: string;
  newSkills: string[];
}

interface RawRegistryInstallResult {
  url: string;
  stdout: string;
  stderr: string;
  new_skills: string[];
}

export interface RegistryUninstallResult {
  name: string;
  removedPath: string;
  scope: string;
}

interface RawRegistryUninstallResult {
  name: string;
  removed_path: string;
  scope: string;
}

export interface ControllerSchemaSummary {
  namespace: string;
  function: string;
  description: string;
  inputs: Array<Record<string, unknown>>;
  outputs: Array<Record<string, unknown>>;
}

interface Envelope<T> {
  data?: T;
}

function unwrap<T>(response: Envelope<T> | T): T {
  if (response && typeof response === 'object' && 'data' in response) {
    const env = response as Envelope<T>;
    if (env.data !== undefined) return env.data as T;
  }
  return response as T;
}

export const skillRegistryApi = {
  browse: async (forceRefresh = false): Promise<CatalogEntry[]> => {
    log('browse: forceRefresh=%s', forceRefresh);
    const response = await callCoreRpc<
      Envelope<{ entries: CatalogEntry[] }> | { entries: CatalogEntry[] }
    >({
      method: 'openhuman.skill_registry_browse',
      params: { force_refresh: forceRefresh },
      timeoutMs: CATALOG_RPC_TIMEOUT_MS,
    });
    const result = unwrap(response);
    log('browse: count=%d', result.entries.length);
    return result.entries;
  },

  search: async (query: string, source?: string, category?: string): Promise<CatalogEntry[]> => {
    log('search: query=%s source=%s category=%s', query, source, category);
    const response = await callCoreRpc<
      Envelope<{ entries: CatalogEntry[] }> | { entries: CatalogEntry[] }
    >({
      method: 'openhuman.skill_registry_search',
      params: { query, ...(source ? { source } : {}), ...(category ? { category } : {}) },
      timeoutMs: CATALOG_RPC_TIMEOUT_MS,
    });
    const result = unwrap(response);
    log('search: count=%d', result.entries.length);
    return result.entries;
  },

  sources: async (): Promise<string[]> => {
    log('sources: request');
    const response = await callCoreRpc<Envelope<{ sources: string[] }> | { sources: string[] }>({
      method: 'openhuman.skill_registry_sources',
      timeoutMs: CATALOG_RPC_TIMEOUT_MS,
    });
    const result = unwrap(response);
    log('sources: count=%d', result.sources.length);
    return result.sources;
  },

  categories: async (): Promise<string[]> => {
    log('categories: request');
    const response = await callCoreRpc<
      Envelope<{ categories: string[] }> | { categories: string[] }
    >({ method: 'openhuman.skill_registry_categories', timeoutMs: CATALOG_RPC_TIMEOUT_MS });
    const result = unwrap(response);
    log('categories: count=%d', result.categories.length);
    return result.categories;
  },

  install: async (entryId: string): Promise<RegistryInstallResult> => {
    log('install: entryId=%s', entryId);
    const response = await callCoreRpc<
      Envelope<RawRegistryInstallResult> | RawRegistryInstallResult
    >({ method: 'openhuman.skill_registry_install', params: { entry_id: entryId } });
    const raw = unwrap(response);
    const result: RegistryInstallResult = {
      url: raw.url,
      stdout: raw.stdout,
      stderr: raw.stderr,
      newSkills: raw.new_skills ?? [],
    };
    log('install: newSkills=%d', result.newSkills.length);
    return result;
  },

  uninstall: async (name: string): Promise<RegistryUninstallResult> => {
    log('uninstall: name=%s', name);
    const response = await callCoreRpc<
      Envelope<RawRegistryUninstallResult> | RawRegistryUninstallResult
    >({ method: 'openhuman.skill_registry_uninstall', params: { name } });
    const raw = unwrap(response);
    const result: RegistryUninstallResult = {
      name: raw.name,
      removedPath: raw.removed_path,
      scope: raw.scope,
    };
    log('uninstall: removedPath=%s', result.removedPath);
    return result;
  },

  schemas: async (): Promise<ControllerSchemaSummary[]> => {
    log('schemas: request');
    const response = await callCoreRpc<
      Envelope<{ schemas: ControllerSchemaSummary[] }> | { schemas: ControllerSchemaSummary[] }
    >({ method: 'openhuman.skill_registry_schemas' });
    const result = unwrap(response);
    log('schemas: count=%d', result.schemas.length);
    return result.schemas;
  },
};
