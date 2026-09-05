import * as fs from 'node:fs';
import * as path from 'node:path';
import { describe, expect, test } from 'vitest';

import { CORE_RPC_METHODS, LEGACY_METHOD_ALIASES, normalizeRpcMethod } from '../rpcMethods';

const EXPECTED_REGISTRY_METHODS = {
  youpetRegistryListAgents: 'openhuman.youpet_registry_list_agents',
  youpetRegistryGetAgentVersion: 'openhuman.youpet_registry_get_agent_version',
  youpetRegistryListToolDefinitions: 'openhuman.youpet_registry_list_tool_definitions',
  youpetRegistryGetToolDefinitionVersion: 'openhuman.youpet_registry_get_tool_definition_version',
  youpetRegistryListToolEnablements: 'openhuman.youpet_registry_list_tool_enablements',
  youpetRegistryGetToolEnablementVersion: 'openhuman.youpet_registry_get_tool_enablement_version',
  youpetRegistryListConnectorTypes: 'openhuman.youpet_registry_list_connector_types',
  youpetRegistryGetConnectorTypeVersion: 'openhuman.youpet_registry_get_connector_type_version',
  youpetRegistryListConnectorBindings: 'openhuman.youpet_registry_list_connector_bindings',
  youpetRegistryGetConnectorBindingVersion:
    'openhuman.youpet_registry_get_connector_binding_version',
} as const;

function parseCoreRpcMethodsFromSource(): Record<string, string> {
  const source = fs.readFileSync(path.resolve(__dirname, '../rpcMethods.ts'), 'utf8');
  const start = source.indexOf('export const CORE_RPC_METHODS = {');
  const end = source.indexOf('} as const;', start);

  if (start === -1 || end === -1) {
    throw new Error('CORE_RPC_METHODS source block not found');
  }

  const body = source
    .slice(start + 'export const CORE_RPC_METHODS = {'.length, end)
    .split('\n')
    .map(line => line.trim())
    .filter(line => line.length > 0 && !line.startsWith('//'));

  return Object.fromEntries(
    body.map(line => {
      const entry = line.match(/^([A-Za-z0-9]+):\s*(['"])(.+)\2,$/);
      if (!entry) {
        throw new Error(`CORE_RPC_METHODS entry is not a single-line quoted value: ${line}`);
      }
      return [entry[1], entry[3]];
    })
  );
}

describe('rpcMethods catalog', () => {
  describe('normalizeRpcMethod', () => {
    test('resolves all legacy aliases to their canonical core method', () => {
      for (const [legacyMethod, coreMethod] of Object.entries(LEGACY_METHOD_ALIASES)) {
        expect(normalizeRpcMethod(legacyMethod)).toBe(coreMethod);
      }
    });

    test('transforms auth methods by replacing dots with underscores', () => {
      expect(normalizeRpcMethod('openhuman.auth.login')).toBe('openhuman.auth_login');
      expect(normalizeRpcMethod('openhuman.auth.get.state')).toBe('openhuman.auth_get_state');
      expect(normalizeRpcMethod('openhuman.auth.a.b.c')).toBe('openhuman.auth_a_b_c');
    });

    test('returns unmapped or unrecognized methods unchanged', () => {
      expect(normalizeRpcMethod('openhuman.threads_list')).toBe('openhuman.threads_list');
      expect(normalizeRpcMethod('openhuman.unknown_method')).toBe('openhuman.unknown_method');
      expect(normalizeRpcMethod('')).toBe('');
      expect(normalizeRpcMethod('random_string')).toBe('random_string');
    });

    test('trims whitespace and converts to lower case', () => {
      expect(normalizeRpcMethod('  OpenHuman.Auth.Login  ')).toBe('openhuman.auth_login');
      expect(normalizeRpcMethod('  OPENHUMAN.GET_CONFIG ')).toBe(CORE_RPC_METHODS.configGet);
      expect(normalizeRpcMethod('OpenHuman.Unrecognized_Status  ')).toBe(
        'openhuman.unrecognized_status'
      );
      expect(normalizeRpcMethod('   some_RANDOM_method  ')).toBe('some_random_method');
    });
  });

  test('legacy aliases point at canonical method values', () => {
    expect(LEGACY_METHOD_ALIASES['openhuman.update_model_settings']).toBe(
      CORE_RPC_METHODS.inferenceUpdateModelSettings
    );
    expect(LEGACY_METHOD_ALIASES['openhuman.workspace_onboarding_flag_set']).toBe(
      CORE_RPC_METHODS.configWorkspaceOnboardingFlagSet
    );
  });

  test('registers all ten Core Registries RPC methods with the Rust spellings', () => {
    expect(
      Object.fromEntries(
        Object.keys(EXPECTED_REGISTRY_METHODS).map(key => [
          key,
          CORE_RPC_METHODS[key as keyof typeof EXPECTED_REGISTRY_METHODS],
        ])
      )
    ).toEqual(EXPECTED_REGISTRY_METHODS);
  });

  test('keeps every CORE_RPC_METHODS entry parseable as a single-line quoted source value', () => {
    expect(parseCoreRpcMethodsFromSource()).toEqual(CORE_RPC_METHODS);
  });

  describe('MCP client legacy alias resolution (Sentry CORE-RUST-DW/DV/DT/DS/DR)', () => {
    test('mcp_clients.list resolves to mcp_clients_installed_list', () => {
      expect(normalizeRpcMethod('mcp_clients.list')).toBe(CORE_RPC_METHODS.mcpClientsInstalledList);
    });

    test('openhuman.mcp_clients_list resolves to mcp_clients_installed_list', () => {
      expect(normalizeRpcMethod('openhuman.mcp_clients_list')).toBe(
        CORE_RPC_METHODS.mcpClientsInstalledList
      );
    });

    test('openhuman.mcp_list resolves to mcp_clients_installed_list', () => {
      expect(normalizeRpcMethod('openhuman.mcp_list')).toBe(
        CORE_RPC_METHODS.mcpClientsInstalledList
      );
    });

    test('openhuman.mcp_servers_list resolves to mcp_clients_installed_list', () => {
      expect(normalizeRpcMethod('openhuman.mcp_servers_list')).toBe(
        CORE_RPC_METHODS.mcpClientsInstalledList
      );
    });

    test('openhuman.tool_registry_call resolves to mcp_clients_tool_call', () => {
      expect(normalizeRpcMethod('openhuman.tool_registry_call')).toBe(
        CORE_RPC_METHODS.mcpClientsToolCall
      );
    });

    test('dotted tool_registry.diagnostics resolves to the canonical method (#3294)', () => {
      expect(normalizeRpcMethod('tool_registry.diagnostics')).toBe(
        CORE_RPC_METHODS.toolRegistryDiagnostics
      );
      expect(CORE_RPC_METHODS.toolRegistryDiagnostics).toBe('openhuman.tool_registry_diagnostics');
    });

    test('canonical mcp_clients_installed_list passes through unchanged', () => {
      expect(normalizeRpcMethod('openhuman.mcp_clients_installed_list')).toBe(
        'openhuman.mcp_clients_installed_list'
      );
    });

    test('canonical mcp_clients_tool_call passes through unchanged', () => {
      expect(normalizeRpcMethod('openhuman.mcp_clients_tool_call')).toBe(
        'openhuman.mcp_clients_tool_call'
      );
    });
  });

  describe('health legacy alias resolution (Sentry CORE-RUST-FG / CORE-RUST-G0)', () => {
    test('health_snapshot resolves to openhuman.health_snapshot', () => {
      expect(normalizeRpcMethod('health_snapshot')).toBe(CORE_RPC_METHODS.healthSnapshot);
    });

    test('openhuman.system_info resolves to openhuman.health_system_info (Sentry CORE-RUST-G0)', () => {
      // Older clients called openhuman.system_info before the method was
      // namespaced under health as openhuman.health_system_info.
      expect(normalizeRpcMethod('openhuman.system_info')).toBe(CORE_RPC_METHODS.healthSystemInfo);
    });

    test('canonical health_system_info passes through unchanged', () => {
      expect(normalizeRpcMethod('openhuman.health_system_info')).toBe(
        'openhuman.health_system_info'
      );
    });
  });

  describe('channels legacy alias resolution (Sentry OPENHUMAN-CORE-1Y / OPENHUMAN-CORE-1Z)', () => {
    test('dotted channel list aliases resolve to channels_list', () => {
      expect(normalizeRpcMethod('channels.list')).toBe(CORE_RPC_METHODS.channelsList);
      expect(normalizeRpcMethod('openhuman.channels.list')).toBe(CORE_RPC_METHODS.channelsList);
    });

    test('canonical channels_list passes through unchanged', () => {
      expect(normalizeRpcMethod('openhuman.channels_list')).toBe('openhuman.channels_list');
    });
  });

  test('catalog canonical methods exist in core schema registry (drift guard)', () => {
    // Read a schema file PLUS every `*_part_*.rs` sibling in its directory.
    //
    // The repo's 750-line layout limit keeps splitting schema files into
    // `include!`-stitched parts (schema_defs.rs → schemas_schema_part_01.rs,
    // inference/schemas.rs → schemas_part_01.rs, …), and each split silently
    // moved the `function: "…"` literals this guard greps out of the file it
    // was reading. Sweeping the siblings makes the corpus follow the splits.
    // Over-inclusion is harmless — the guard only searches for substrings —
    // and `readFileSync` still throws if a listed base file moves entirely,
    // which is the loud failure we want.
    const readWithParts = (relFile: string): string => {
      const abs = path.resolve(__dirname, relFile);
      const dir = path.dirname(abs);
      const parts = fs
        .readdirSync(dir)
        .filter(name => name.endsWith('.rs') && name.includes('_part_'))
        .sort()
        .map(name => fs.readFileSync(path.join(dir, name), 'utf8'));
      return [fs.readFileSync(abs, 'utf8'), ...parts].join('\n');
    };

    const configSchemaSource = readWithParts(
      '../../../../src/openhuman/config/schemas/schema_defs.rs'
    );
    const inferenceProviderSchemaSource = readWithParts(
      '../../../../src/openhuman/inference/provider/schemas.rs'
    );
    const inferenceSchemaSource = readWithParts('../../../../src/openhuman/inference/schemas.rs');
    const inferenceLocalSchemaSource = readWithParts(
      '../../../../src/openhuman/inference/local/schemas.rs'
    );
    const embeddingsSchemaSource = readWithParts(
      '../../../../src/openhuman/inference/embeddings/schemas.rs'
    );
    const mcpRegistrySchemaSource = readWithParts(
      '../../../../src/openhuman/mcp/registry/schemas.rs'
    );
    const toolRegistrySchemaSource = readWithParts(
      '../../../../src/openhuman/tools/registry/schemas.rs'
    );
    const healthSchemaSource = readWithParts(
      '../../../../src/openhuman/platform/health/schemas.rs'
    );
    const youpetSchemaSource = readWithParts('../../../../src/openhuman/youpet/schemas.rs');
    const youpetRegistrySchemaSource = readWithParts(
      '../../../../src/openhuman/youpet/registry/schemas.rs'
    );
    const channelsSchemaSource = readWithParts(
      '../../../../src/openhuman/channels/controllers/schemas.rs'
    );
    const schemaSources = [
      configSchemaSource,
      inferenceProviderSchemaSource,
      inferenceSchemaSource,
      inferenceLocalSchemaSource,
      embeddingsSchemaSource,
      mcpRegistrySchemaSource,
      toolRegistrySchemaSource,
      healthSchemaSource,
      youpetSchemaSource,
      youpetRegistrySchemaSource,
      channelsSchemaSource,
    ].join('\n');

    for (const method of Object.values(CORE_RPC_METHODS)) {
      // core.* methods (e.g. core.ping) are special dispatch methods, not in the schema catalog.
      if (!method.startsWith('openhuman.')) continue;
      const methodRoot = method.slice('openhuman.'.length);
      const namespace = methodRoot.startsWith('inference_')
        ? 'inference'
        : methodRoot.startsWith('embeddings_')
          ? 'embeddings'
          : methodRoot.startsWith('providers_')
            ? 'providers'
            : methodRoot.startsWith('mcp_clients_')
              ? 'mcp_clients'
              : methodRoot.startsWith('health_')
                ? 'health'
                : methodRoot.startsWith('channels_')
                  ? 'channels'
                  : methodRoot.startsWith('youpet_registry_')
                    ? 'youpet'
                    : methodRoot.startsWith('youpet_')
                      ? 'youpet'
                      : methodRoot.startsWith('tool_registry_')
                        ? 'tool_registry'
                        : 'config';
      const fnName = methodRoot.slice(`${namespace}_`.length);
      if (namespace === 'channels') {
        expect(channelsSchemaSource).toContain(`schemas("${fnName}")`);
      } else {
        expect(schemaSources).toContain(`namespace: "${namespace}"`);
        expect(schemaSources).toContain(`function: "${fnName}"`);
      }
    }
  });
});
