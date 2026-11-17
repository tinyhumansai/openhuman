/**
 * Adding a hosted directory server to `mcp.json` in one step.
 *
 * Only a server that can be dialled as declared qualifies: one hosted
 * endpoint with a concrete http(s) URL and nothing for the user to fill in.
 * Anything else keeps the "open its page" path, where the user reads the
 * install instructions and declares it by hand.
 */
import debug from 'debug';

import { mcpClientsApi } from '../../../services/api/mcpClientsApi';
import type { SmitheryServer, SmitheryServerDetail } from './types';

const log = debug('mcp-clients:registry-add');

const HOSTED_TYPES = new Set(['http', 'sse']);
const TEMPLATE_VAR = /\{[^}]*\}/;

export type HostedEntry = { url: string; description?: string };

export type ResolveRefusal =
  | 'no_hosted_connection'
  | 'missing_url'
  | 'templated_url'
  | 'invalid_url'
  | 'unsupported_scheme'
  | 'multiple_urls'
  | 'requires_input'
  | 'secret_input'
  | 'requires_env';

export type ResolveResult =
  | { ok: true; entry: HostedEntry }
  | { ok: false; reason: ResolveRefusal };

export type DeclareOutcome = 'added' | 'exists';

/** Whether a directory row is worth offering the one-step Add for. */
export const isAddCandidate = (server: SmitheryServer): boolean =>
  server.is_deployed === true && server.source !== 'smithery' && server.auth_kind !== 'api_key';

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

const schemaRefusal = (schema: unknown): ResolveRefusal | null => {
  if (!isRecord(schema)) return null;
  const required = schema.required;
  if (Array.isArray(required) && required.length > 0) return 'requires_input';
  const properties = schema.properties;
  if (isRecord(properties)) {
    for (const property of Object.values(properties)) {
      if (!isRecord(property)) continue;
      if (property['x-secret'] === true || property.isSecret === true) return 'secret_input';
    }
  }
  return null;
};

const urlRefusal = (raw: string | undefined): ResolveRefusal | null => {
  const url = raw?.trim();
  if (!url) return 'missing_url';
  if (TEMPLATE_VAR.test(url)) return 'templated_url';
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return 'invalid_url';
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return 'unsupported_scheme';
  return null;
};

/**
 * The `mcp.json` entry a directory server would be declared with, or why it
 * cannot be declared without the user's input.
 */
export const resolveHostedEntry = (
  detail: SmitheryServerDetail,
  server: SmitheryServer
): ResolveResult => {
  const refuse = (reason: ResolveRefusal): ResolveResult => {
    log('resolve %s refused: %s', server.qualified_name, reason);
    return { ok: false, reason };
  };

  const hosted = (detail.connections ?? []).filter(c => HOSTED_TYPES.has(String(c.type)));
  if (hosted.length === 0) return refuse('no_hosted_connection');

  const urls = new Set<string>();
  for (const connection of hosted) {
    const refusal =
      urlRefusal(connection.deployment_url) ?? schemaRefusal(connection.config_schema);
    if (refusal) return refuse(refusal);
    urls.add((connection.deployment_url as string).trim());
  }
  if (urls.size !== 1) return refuse('multiple_urls');
  if ((detail.required_env_keys ?? []).length > 0) return refuse('requires_env');

  const [url] = urls;
  const description = (server.description ?? detail.description)?.trim();
  return { ok: true, entry: description ? { url, description } : { url } };
};

/**
 * Declare `entry` under `name` in `mcp.json`, keeping every other entry as it
 * is. An existing entry of that name is left untouched. A refusal from the
 * core is rethrown as it came.
 */
export const declareServer = async (name: string, entry: HostedEntry): Promise<DeclareOutcome> => {
  const doc = await mcpClientsApi.configGet();
  const servers = doc.mcpServers ?? {};
  if (Object.prototype.hasOwnProperty.call(servers, name)) {
    log('declare %s skipped: already declared', name);
    return 'exists';
  }
  await mcpClientsApi.configSet({ mcpServers: { ...servers, [name]: entry } });
  log('declare %s added', name);
  return 'added';
};
