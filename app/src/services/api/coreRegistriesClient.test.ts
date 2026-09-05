import { beforeEach, describe, expect, expectTypeOf, it, vi } from 'vitest';

import { CoreRpcError } from '../coreRpcClient';
import {
  type AgentRegistryAgent,
  type AgentRegistryAgentSummary,
  type ConnectorRegistryBinding,
  type ConnectorRegistryBindingSummary,
  type ConnectorRegistryType,
  type ConnectorRegistryTypeSummary,
  coreRegistriesClient,
  type CursorRegistryPage,
  extractRegistryBridgeErrorMeta,
  type RegistryBridgeErrorKind,
  type RegistryCursorListParams,
  type ToolRegistryToolDefinition,
  type ToolRegistryToolDefinitionSummary,
  type ToolRegistryToolEnablement,
  type UnpagedRegistryCollection,
} from './coreRegistriesClient';

vi.mock('../coreRpcClient', async () => {
  const actual = await vi.importActual<typeof import('../coreRpcClient')>('../coreRpcClient');
  return { ...actual, callCoreRpc: vi.fn() };
});

describe('coreRegistriesClient contracts', () => {
  beforeEach(async () => {
    const { callCoreRpc } = await import('../coreRpcClient');
    vi.mocked(callCoreRpc).mockReset();
  });

  it('freezes list/exact method param surfaces', () => {
    expectTypeOf<RegistryCursorListParams>().toMatchTypeOf<{ limit?: number; cursor?: string }>();
    expectTypeOf<Extract<keyof RegistryCursorListParams, 'tenantId'>>().toEqualTypeOf<never>();
    expectTypeOf<Extract<keyof RegistryCursorListParams, 'coreUrl'>>().toEqualTypeOf<never>();
    expectTypeOf<Extract<keyof RegistryCursorListParams, 'token'>>().toEqualTypeOf<never>();
    expectTypeOf<Extract<keyof RegistryCursorListParams, 'headers'>>().toEqualTypeOf<never>();
    expectTypeOf<Extract<keyof RegistryCursorListParams, 'path'>>().toEqualTypeOf<never>();
    expectTypeOf<Extract<keyof RegistryCursorListParams, 'method'>>().toEqualTypeOf<never>();

    expectTypeOf<typeof coreRegistriesClient.listAgents>().parameters.toEqualTypeOf<
      [params?: RegistryCursorListParams]
    >();
    expectTypeOf<typeof coreRegistriesClient.listToolDefinitions>().parameters.toEqualTypeOf<
      [params?: RegistryCursorListParams]
    >();
    expectTypeOf<typeof coreRegistriesClient.listConnectorTypes>().parameters.toEqualTypeOf<
      [params?: RegistryCursorListParams]
    >();
    expectTypeOf<typeof coreRegistriesClient.listConnectorBindings>().parameters.toEqualTypeOf<
      [params?: RegistryCursorListParams]
    >();
    expectTypeOf<typeof coreRegistriesClient.listToolEnablements>().parameters.toEqualTypeOf<[]>();

    expectTypeOf<typeof coreRegistriesClient.getAgentVersion>().parameters.toEqualTypeOf<
      [{ agentKey: string; version: number }]
    >();
    expectTypeOf<typeof coreRegistriesClient.getToolDefinitionVersion>().parameters.toEqualTypeOf<
      [{ toolKey: string; version: number }]
    >();
    expectTypeOf<typeof coreRegistriesClient.getToolEnablementVersion>().parameters.toEqualTypeOf<
      [{ toolKey: string; version: number }]
    >();
    expectTypeOf<typeof coreRegistriesClient.getConnectorTypeVersion>().parameters.toEqualTypeOf<
      [{ connectorKey: string; version: number }]
    >();
    expectTypeOf<typeof coreRegistriesClient.getConnectorBindingVersion>().parameters.toEqualTypeOf<
      [{ bindingKey: string; version: number }]
    >();
  });

  it('uses callCoreRpc for the four cursor-backed lists', async () => {
    const { callCoreRpc } = await import('../coreRpcClient');
    vi.mocked(callCoreRpc)
      .mockResolvedValueOnce({ items: [], next_cursor: null })
      .mockResolvedValueOnce({ items: [], next_cursor: 'tool-cursor' })
      .mockResolvedValueOnce({ items: [], next_cursor: 'connector-type-cursor' })
      .mockResolvedValueOnce({ items: [], next_cursor: 'binding-cursor' });

    await expect(
      coreRegistriesClient.listAgents({ limit: 25, cursor: 'agent-cursor' })
    ).resolves.toEqual({
      items: [],
      nextCursor: null,
    } satisfies CursorRegistryPage<AgentRegistryAgentSummary>);
    await expect(coreRegistriesClient.listToolDefinitions()).resolves.toEqual({
      items: [],
      nextCursor: 'tool-cursor',
    } satisfies CursorRegistryPage<ToolRegistryToolDefinitionSummary>);
    await expect(
      coreRegistriesClient.listConnectorTypes({ cursor: 'connector-type-cursor' })
    ).resolves.toEqual({
      items: [],
      nextCursor: 'connector-type-cursor',
    } satisfies CursorRegistryPage<ConnectorRegistryTypeSummary>);
    await expect(coreRegistriesClient.listConnectorBindings({ limit: 10 })).resolves.toEqual({
      items: [],
      nextCursor: 'binding-cursor',
    } satisfies CursorRegistryPage<ConnectorRegistryBindingSummary>);

    expect(callCoreRpc).toHaveBeenNthCalledWith(1, {
      method: 'openhuman.youpet_registry_list_agents',
      params: { limit: 25, cursor: 'agent-cursor' },
    });
    expect(callCoreRpc).toHaveBeenNthCalledWith(2, {
      method: 'openhuman.youpet_registry_list_tool_definitions',
      params: {},
    });
    expect(callCoreRpc).toHaveBeenNthCalledWith(3, {
      method: 'openhuman.youpet_registry_list_connector_types',
      params: { cursor: 'connector-type-cursor' },
    });
    expect(callCoreRpc).toHaveBeenNthCalledWith(4, {
      method: 'openhuman.youpet_registry_list_connector_bindings',
      params: { limit: 10 },
    });
  });

  it('unwraps the logged Rust RpcOutcome envelope before registry decoding', async () => {
    const { callCoreRpc } = await import('../coreRpcClient');
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      result: { items: [], next_cursor: null },
      logs: ['[youpet] listed Core registry agents'],
    });

    await expect(coreRegistriesClient.listAgents()).resolves.toEqual({
      items: [],
      nextCursor: null,
    });
  });

  it('uses items-only decoding for the unpaged tool enablement list', async () => {
    const { callCoreRpc } = await import('../coreRpcClient');
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      items: [
        {
          tool_key: 'tool.alpha',
          version: 3,
          lifecycle_state: 'enabled',
          generation: 11,
          timeout_cap_ms: 5000,
          approval_required: true,
          allow_ttl_seconds: 120,
          audit_mode: 'metadata_only',
          updated_at: '2026-09-01T12:00:00Z',
        },
      ],
      next_cursor: 'must-be-ignored',
    });

    await expect(coreRegistriesClient.listToolEnablements()).resolves.toEqual({
      items: [
        {
          toolKey: 'tool.alpha',
          version: 3,
          lifecycleState: 'enabled',
          generation: 11,
          timeoutCapMs: 5000,
          approvalRequired: true,
          allowTtlSeconds: 120,
          auditMode: 'metadata_only',
          updatedAt: '2026-09-01T12:00:00Z',
        },
      ],
    } satisfies UnpagedRegistryCollection<ToolRegistryToolEnablement>);

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.youpet_registry_list_tool_enablements',
      params: {},
    });
  });

  it('rejects cursor-backed responses that omit required nullable next_cursor', async () => {
    const { callCoreRpc } = await import('../coreRpcClient');
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ items: [] });
    const operation = coreRegistriesClient.listAgents();

    await expect(operation).rejects.toMatchObject({
      name: 'CoreRpcError',
      data: { kind: 'YouPetCoreResponseShape' },
    });
    await expect(operation.catch(extractRegistryBridgeErrorMeta)).resolves.toEqual({
      kind: 'YouPetCoreResponseShape',
    });
  });

  it('rejects non-active rows from active-only agent and tool definition lists', async () => {
    const { callCoreRpc } = await import('../coreRpcClient');
    vi.mocked(callCoreRpc)
      .mockResolvedValueOnce({
        items: [
          {
            id: 'agent-record',
            agent_key: 'agent.alpha',
            version: 7,
            lifecycle_state: 'draft',
            configuration_fingerprint: 'a'.repeat(64),
            owner_actor_type: 'service',
            owner_actor_id: 'registry-reader',
            created_at: '2026-09-01T12:00:00Z',
          },
        ],
        next_cursor: null,
      })
      .mockResolvedValueOnce({
        items: [
          {
            tool_key: 'tool.alpha',
            version: 3,
            lifecycle_state: 'retired',
            definition_fingerprint: 'b'.repeat(64),
            schema_version: 1,
            display_name: 'Tool Alpha',
            description: 'Reads data',
            tool_effect_class: 'read_only',
            abstract_auth_scopes: ['scope.read'],
            created_at: '2026-09-01T12:05:00Z',
          },
        ],
        next_cursor: null,
      });

    await expect(coreRegistriesClient.listAgents()).rejects.toMatchObject({
      name: 'CoreRpcError',
      data: { kind: 'YouPetCoreResponseShape' },
    });
    await expect(coreRegistriesClient.listToolDefinitions()).rejects.toMatchObject({
      name: 'CoreRpcError',
      data: { kind: 'YouPetCoreResponseShape' },
    });
  });

  it('classifies malformed exact-record shapes as YouPetCoreResponseShape', async () => {
    const { callCoreRpc } = await import('../coreRpcClient');
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      toolDefinition: {
        tool_key: 'tool.alpha',
        version: 3,
        lifecycle_state: 'active',
        definition_fingerprint: 'b'.repeat(64),
        schema_version: 1,
        display_name: 'Tool Alpha',
        description: 'Reads data',
        tool_effect_class: 'read_only',
        abstract_auth_scopes: ['scope.read'],
        input_schema: { type: 'object' },
        output_schema: { type: 'object' },
        timeout_defaults: { ms: 5000 },
        retry_contract: { attempts: 2 },
        audit_contract: { mode: 'metadata_only' },
      },
    });

    const operation = coreRegistriesClient.getToolDefinitionVersion({
      toolKey: 'tool.alpha',
      version: 3,
    });

    await expect(operation).rejects.toMatchObject({
      name: 'CoreRpcError',
      data: { kind: 'YouPetCoreResponseShape' },
    });
    await expect(operation.catch(extractRegistryBridgeErrorMeta)).resolves.toEqual({
      kind: 'YouPetCoreResponseShape',
    });
  });

  it('loads exact records through domain-specific key/version methods', async () => {
    const { callCoreRpc } = await import('../coreRpcClient');
    vi.mocked(callCoreRpc)
      .mockResolvedValueOnce({
        agent: {
          id: 'agent-record',
          agent_key: 'agent.alpha',
          version: 7,
          lifecycle_state: 'active',
          configuration: {
            schema_version: 1,
            domain_key: 'ops',
            owner: { actor_type: 'service', actor_id: 'registry-reader' },
            allowed_tool_refs: [{ tool_key: 'tool.alpha', version: 3 }],
            knowledge_scope_refs: [
              { source_key: 'docs', trust_version: 'v1', access_scope: 'read' },
            ],
            risk_policy_ref: { policy_id: 'risk.default', policy_version: 'v2' },
          },
          configuration_fingerprint: 'a'.repeat(64),
          owner_actor_type: 'service',
          owner_actor_id: 'registry-reader',
          created_at: '2026-09-01T12:00:00Z',
        },
      })
      .mockResolvedValueOnce({
        toolDefinition: {
          tool_key: 'tool.alpha',
          version: 3,
          lifecycle_state: 'active',
          definition_fingerprint: 'b'.repeat(64),
          schema_version: 1,
          display_name: 'Tool Alpha',
          description: 'Reads data',
          tool_effect_class: 'read_only',
          abstract_auth_scopes: ['scope.read'],
          input_schema: { type: 'object' },
          output_schema: { type: 'object' },
          timeout_defaults: { ms: 5000 },
          retry_contract: { attempts: 2 },
          audit_contract: { mode: 'metadata_only' },
          created_at: '2026-09-01T12:05:00Z',
        },
      })
      .mockResolvedValueOnce({
        toolEnablement: {
          tool_key: 'tool.alpha',
          version: 5,
          lifecycle_state: 'disabled',
          generation: 12,
          timeout_cap_ms: null,
          approval_required: false,
          allow_ttl_seconds: null,
          audit_mode: null,
          updated_at: '2026-09-01T12:10:00Z',
        },
      })
      .mockResolvedValueOnce({
        connectorType: {
          connector_key: 'wecom',
          version: 2,
          lifecycle_state: 'active',
          source_type: 'wecom',
          connector_type_fingerprint: 'c'.repeat(64),
          capabilities: ['messages.read'],
          normalization_contracts: [
            {
              evidence_family: 'messages',
              kernel_event_type: 'message.created',
              kernel_event_schema_version: 1,
            },
          ],
          delivery_behavior: { mode: 'push' },
          created_at: '2026-09-01T12:15:00Z',
        },
      })
      .mockResolvedValueOnce({
        connectorBinding: {
          binding_key: 'wecom-primary',
          version: 11,
          lifecycle_state: 'retired',
          connector_type_key: 'wecom',
          connector_type_version: 2,
          connector_type_fingerprint: 'd'.repeat(64),
          provider_account: { namespace: 'wechat', external_account_ref: 'acct-1' },
          config_ref: 'config://wecom/primary',
          credential_ref: 'credential://wecom/primary',
          enabled_capabilities: ['messages.read'],
          binding_fingerprint: 'e'.repeat(64),
          created_at: '2026-09-01T12:20:00Z',
        },
      });

    await expect(
      coreRegistriesClient.getAgentVersion({ agentKey: 'agent.alpha', version: 7 })
    ).resolves.toEqual({
      id: 'agent-record',
      agentKey: 'agent.alpha',
      version: 7,
      lifecycleState: 'active',
      configuration: {
        schemaVersion: 1,
        domainKey: 'ops',
        owner: { actorType: 'service', actorId: 'registry-reader' },
        allowedToolRefs: [{ toolKey: 'tool.alpha', version: 3 }],
        knowledgeScopeRefs: [{ sourceKey: 'docs', trustVersion: 'v1', accessScope: 'read' }],
        riskPolicyRef: { policyId: 'risk.default', policyVersion: 'v2' },
      },
      configurationFingerprint: 'a'.repeat(64),
      ownerActorType: 'service',
      ownerActorId: 'registry-reader',
      createdAt: '2026-09-01T12:00:00Z',
    } satisfies AgentRegistryAgent);
    await expect(
      coreRegistriesClient.getToolDefinitionVersion({ toolKey: 'tool.alpha', version: 3 })
    ).resolves.toEqual({
      toolKey: 'tool.alpha',
      version: 3,
      lifecycleState: 'active',
      definitionFingerprint: 'b'.repeat(64),
      schemaVersion: 1,
      displayName: 'Tool Alpha',
      description: 'Reads data',
      toolEffectClass: 'read_only',
      abstractAuthScopes: ['scope.read'],
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
      timeoutDefaults: { ms: 5000 },
      retryContract: { attempts: 2 },
      auditContract: { mode: 'metadata_only' },
      createdAt: '2026-09-01T12:05:00Z',
    } satisfies ToolRegistryToolDefinition);
    await expect(
      coreRegistriesClient.getToolEnablementVersion({ toolKey: 'tool.alpha', version: 5 })
    ).resolves.toEqual({
      toolKey: 'tool.alpha',
      version: 5,
      lifecycleState: 'disabled',
      generation: 12,
      timeoutCapMs: null,
      approvalRequired: false,
      allowTtlSeconds: null,
      auditMode: null,
      updatedAt: '2026-09-01T12:10:00Z',
    } satisfies ToolRegistryToolEnablement);
    await expect(
      coreRegistriesClient.getConnectorTypeVersion({ connectorKey: 'wecom', version: 2 })
    ).resolves.toEqual({
      connectorKey: 'wecom',
      version: 2,
      lifecycleState: 'active',
      sourceType: 'wecom',
      connectorTypeFingerprint: 'c'.repeat(64),
      capabilities: ['messages.read'],
      normalizationContracts: [
        {
          evidenceFamily: 'messages',
          kernelEventType: 'message.created',
          kernelEventSchemaVersion: 1,
        },
      ],
      deliveryBehavior: { mode: 'push' },
      createdAt: '2026-09-01T12:15:00Z',
    } satisfies ConnectorRegistryType);
    await expect(
      coreRegistriesClient.getConnectorBindingVersion({ bindingKey: 'wecom-primary', version: 11 })
    ).resolves.toEqual({
      bindingKey: 'wecom-primary',
      version: 11,
      lifecycleState: 'retired',
      connectorTypeKey: 'wecom',
      connectorTypeVersion: 2,
      connectorTypeFingerprint: 'd'.repeat(64),
      providerAccount: { namespace: 'wechat', externalAccountRef: 'acct-1' },
      configRef: 'config://wecom/primary',
      credentialRef: 'credential://wecom/primary',
      enabledCapabilities: ['messages.read'],
      bindingFingerprint: 'e'.repeat(64),
      createdAt: '2026-09-01T12:20:00Z',
    } satisfies ConnectorRegistryBinding);

    expect(callCoreRpc).toHaveBeenNthCalledWith(1, {
      method: 'openhuman.youpet_registry_get_agent_version',
      params: { agentKey: 'agent.alpha', version: 7 },
    });
    expect(callCoreRpc).toHaveBeenNthCalledWith(2, {
      method: 'openhuman.youpet_registry_get_tool_definition_version',
      params: { toolKey: 'tool.alpha', version: 3 },
    });
    expect(callCoreRpc).toHaveBeenNthCalledWith(3, {
      method: 'openhuman.youpet_registry_get_tool_enablement_version',
      params: { toolKey: 'tool.alpha', version: 5 },
    });
    expect(callCoreRpc).toHaveBeenNthCalledWith(4, {
      method: 'openhuman.youpet_registry_get_connector_type_version',
      params: { connectorKey: 'wecom', version: 2 },
    });
    expect(callCoreRpc).toHaveBeenNthCalledWith(5, {
      method: 'openhuman.youpet_registry_get_connector_binding_version',
      params: { bindingKey: 'wecom-primary', version: 11 },
    });
  });

  it('extracts only safe registry bridge error metadata for Q23-Q27 classes', () => {
    const errorCases: Array<{
      error: CoreRpcError;
      expected: {
        kind: RegistryBridgeErrorKind;
        httpStatus?: number;
        coreCode?: string;
        retryAfterSeconds?: number;
      };
    }> = [
      {
        error: new CoreRpcError('config missing', 'unknown', undefined, {
          kind: 'YouPetConfigMissing',
          youpet: { field: 'service_token', secret: 'must-not-leak' },
        }),
        expected: { kind: 'YouPetConfigMissing' },
      },
      {
        error: new CoreRpcError('forbidden', 'unknown', undefined, {
          kind: 'YouPetCoreHttpError',
          youpet: {
            http_status: 403,
            code: 'forbidden_actor',
            raw_body: { detail: 'must-not-leak' },
          },
        }),
        expected: { kind: 'YouPetCoreHttpError', httpStatus: 403, coreCode: 'forbidden_actor' },
      },
      {
        error: new CoreRpcError('tenant unavailable', 'unknown', undefined, {
          kind: 'YouPetCoreHttpError',
          youpet: { http_status: 503, code: 'kernel_tenant_unavailable' },
        }),
        expected: {
          kind: 'YouPetCoreHttpError',
          httpStatus: 503,
          coreCode: 'kernel_tenant_unavailable',
        },
      },
      {
        error: new CoreRpcError('invalid cursor', 'unknown', undefined, {
          kind: 'YouPetCoreHttpError',
          youpet: { http_status: 422, code: 'invalid_cursor', cursor: 'secret-cursor' },
        }),
        expected: { kind: 'YouPetCoreHttpError', httpStatus: 422, coreCode: 'invalid_cursor' },
      },
      {
        error: new CoreRpcError('rate limited', 'unknown', undefined, {
          kind: 'YouPetCoreHttpError',
          youpet: {
            http_status: 429,
            code: 'rate_limited',
            retry_after_seconds: 2,
            response_body: 'must-not-leak',
          },
        }),
        expected: {
          kind: 'YouPetCoreHttpError',
          httpStatus: 429,
          coreCode: 'rate_limited',
          retryAfterSeconds: 2,
        },
      },
      {
        error: new CoreRpcError('detail missing', 'unknown', undefined, {
          kind: 'YouPetCoreHttpError',
          youpet: { http_status: 404, code: 'tool_definition_not_found' },
        }),
        expected: {
          kind: 'YouPetCoreHttpError',
          httpStatus: 404,
          coreCode: 'tool_definition_not_found',
        },
      },
      {
        error: new CoreRpcError('schema mismatch', 'unknown', undefined, {
          kind: 'YouPetCoreResponseShape',
          youpet: { raw_body: { secret: 'must-not-leak' } },
        }),
        expected: { kind: 'YouPetCoreResponseShape' },
      },
    ];

    for (const { error, expected } of errorCases) {
      expect(extractRegistryBridgeErrorMeta(error)).toEqual(expected);
    }

    expect(extractRegistryBridgeErrorMeta(new Error('plain error'))).toBeNull();
  });
});
