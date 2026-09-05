import { z } from 'zod';

import { callCoreRpc, CoreRpcError } from '../coreRpcClient';
import { CORE_RPC_METHODS } from '../rpcMethods';

const MAX_REGISTRY_KEY_LENGTH = 128;

const registryOwnerActorTypeSchema = z.enum(['service', 'user']);
const agentLifecycleStateSchema = z.enum(['draft', 'active', 'retired']);
const toolDefinitionLifecycleStateSchema = z.enum(['draft', 'active', 'retired']);
const toolEffectClassSchema = z.enum(['read_only', 'effectful', 'destructive']);
const toolEnablementLifecycleStateSchema = z.enum(['enabled', 'disabled']);
const toolEnablementAuditModeSchema = z.enum(['metadata_only', 'redacted_io']);
const connectorTypeLifecycleStateSchema = z.enum(['draft', 'active', 'retired']);
const connectorBindingLifecycleStateSchema = z.enum(['draft', 'active', 'retired']);
const jsonObjectSchema = z.object({}).catchall(z.unknown());

export type RegistryOwnerActorType = z.infer<typeof registryOwnerActorTypeSchema>;
export type AgentRegistryLifecycleState = z.infer<typeof agentLifecycleStateSchema>;
export type ToolDefinitionLifecycleState = z.infer<typeof toolDefinitionLifecycleStateSchema>;
export type ToolEffectClass = z.infer<typeof toolEffectClassSchema>;
export type ToolEnablementLifecycleState = z.infer<typeof toolEnablementLifecycleStateSchema>;
export type ToolEnablementAuditMode = z.infer<typeof toolEnablementAuditModeSchema>;
export type ConnectorTypeLifecycleState = z.infer<typeof connectorTypeLifecycleStateSchema>;
export type ConnectorBindingLifecycleState = z.infer<typeof connectorBindingLifecycleStateSchema>;

export interface CursorRegistryPage<T> {
  items: T[];
  nextCursor: string | null;
}

export interface UnpagedRegistryCollection<T> {
  items: T[];
}

export interface RegistryCursorListParams {
  limit?: number;
  cursor?: string;
}

export interface AgentOwnerRef {
  actorType: RegistryOwnerActorType;
  actorId: string;
}

export interface ToolRefV1 {
  toolKey: string;
  version: number;
}

export interface KnowledgeScopeRefV1 {
  sourceKey: string;
  trustVersion: string;
  accessScope: string;
}

export interface PolicyRefV1 {
  policyId: string;
  policyVersion: string;
}

export interface AgentConfigurationV1 {
  schemaVersion: number;
  domainKey: string;
  owner: AgentOwnerRef;
  allowedToolRefs: ToolRefV1[];
  knowledgeScopeRefs: KnowledgeScopeRefV1[];
  riskPolicyRef: PolicyRefV1 | null;
}

export interface AgentRegistryAgentSummary {
  id: string;
  agentKey: string;
  version: number;
  lifecycleState: 'active';
  configurationFingerprint: string;
  ownerActorType: RegistryOwnerActorType;
  ownerActorId: string;
  createdAt: string;
}

export interface AgentRegistryAgent extends Omit<AgentRegistryAgentSummary, 'lifecycleState'> {
  lifecycleState: AgentRegistryLifecycleState;
  configuration: AgentConfigurationV1;
}

export interface ToolRegistryToolDefinitionSummary {
  toolKey: string;
  version: number;
  lifecycleState: 'active';
  definitionFingerprint: string;
  schemaVersion: number;
  displayName: string;
  description: string;
  toolEffectClass: ToolEffectClass;
  abstractAuthScopes: string[];
  createdAt: string;
}

export interface ToolRegistryToolDefinition extends Omit<
  ToolRegistryToolDefinitionSummary,
  'lifecycleState'
> {
  lifecycleState: ToolDefinitionLifecycleState;
  inputSchema: Record<string, unknown>;
  outputSchema: Record<string, unknown>;
  timeoutDefaults: Record<string, unknown>;
  retryContract: Record<string, unknown>;
  auditContract: Record<string, unknown>;
}

export interface ToolRegistryToolEnablement {
  toolKey: string;
  version: number;
  lifecycleState: ToolEnablementLifecycleState;
  generation: number;
  timeoutCapMs: number | null;
  approvalRequired: boolean;
  allowTtlSeconds: number | null;
  auditMode: ToolEnablementAuditMode | null;
  updatedAt: string;
}

export interface ConnectorNormalizationContract {
  evidenceFamily: string;
  kernelEventType: string;
  kernelEventSchemaVersion: number;
}

export interface ConnectorRegistryTypeSummary {
  connectorKey: string;
  version: number;
  lifecycleState: ConnectorTypeLifecycleState;
  sourceType: string;
  connectorTypeFingerprint: string;
  capabilities: string[];
  createdAt: string;
}

export interface ConnectorRegistryType extends ConnectorRegistryTypeSummary {
  normalizationContracts: ConnectorNormalizationContract[];
  deliveryBehavior: Record<string, unknown>;
}

export interface ConnectorRegistryProviderAccount {
  namespace: string;
  externalAccountRef: string;
}

export interface ConnectorRegistryBindingSummary {
  bindingKey: string;
  version: number;
  lifecycleState: ConnectorBindingLifecycleState;
  connectorTypeKey: string;
  connectorTypeVersion: number;
  connectorTypeFingerprint: string;
  enabledCapabilities: string[];
  bindingFingerprint: string;
  createdAt: string;
}

export interface ConnectorRegistryBinding extends ConnectorRegistryBindingSummary {
  providerAccount: ConnectorRegistryProviderAccount;
  configRef: string;
  credentialRef: string;
}

export const REGISTRY_BRIDGE_ERROR_KINDS = [
  'YouPetConfigMissing',
  'YouPetConfigInvalid',
  'YouPetRequestInvalid',
  'YouPetCoreTransport',
  'YouPetCoreInvalidJson',
  'YouPetCoreResponseShape',
  'YouPetCoreHttpError',
] as const;

export type RegistryBridgeErrorKind = (typeof REGISTRY_BRIDGE_ERROR_KINDS)[number];

export interface RegistryBridgeErrorMeta {
  kind: RegistryBridgeErrorKind;
  httpStatus?: number;
  coreCode?: string;
  retryAfterSeconds?: number;
}

const agentOwnerRefSchema = z
  .object({ actor_type: registryOwnerActorTypeSchema, actor_id: z.string() })
  .transform(
    ({ actor_type, actor_id }): AgentOwnerRef => ({ actorType: actor_type, actorId: actor_id })
  );

const toolRefSchema = z
  .object({ tool_key: z.string(), version: z.number().int() })
  .transform(({ tool_key, version }): ToolRefV1 => ({ toolKey: tool_key, version }));

const knowledgeScopeRefSchema = z
  .object({ source_key: z.string(), trust_version: z.string(), access_scope: z.string() })
  .transform(
    ({ source_key, trust_version, access_scope }): KnowledgeScopeRefV1 => ({
      sourceKey: source_key,
      trustVersion: trust_version,
      accessScope: access_scope,
    })
  );

const policyRefSchema = z
  .object({ policy_id: z.string(), policy_version: z.string() })
  .transform(
    ({ policy_id, policy_version }): PolicyRefV1 => ({
      policyId: policy_id,
      policyVersion: policy_version,
    })
  );

const agentConfigurationSchema = z
  .object({
    schema_version: z.number().int(),
    domain_key: z.string(),
    owner: agentOwnerRefSchema,
    allowed_tool_refs: z.array(toolRefSchema),
    knowledge_scope_refs: z.array(knowledgeScopeRefSchema),
    risk_policy_ref: policyRefSchema.nullable(),
  })
  .transform(
    ({
      schema_version,
      domain_key,
      owner,
      allowed_tool_refs,
      knowledge_scope_refs,
      risk_policy_ref,
    }): AgentConfigurationV1 => ({
      schemaVersion: schema_version,
      domainKey: domain_key,
      owner,
      allowedToolRefs: allowed_tool_refs,
      knowledgeScopeRefs: knowledge_scope_refs,
      riskPolicyRef: risk_policy_ref,
    })
  );

const agentSummarySchema = z
  .object({
    id: z.string(),
    agent_key: z.string(),
    version: z.number().int(),
    lifecycle_state: z.literal('active'),
    configuration_fingerprint: z.string(),
    owner_actor_type: registryOwnerActorTypeSchema,
    owner_actor_id: z.string(),
    created_at: z.string(),
  })
  .transform(
    ({
      id,
      agent_key,
      version,
      lifecycle_state,
      configuration_fingerprint,
      owner_actor_type,
      owner_actor_id,
      created_at,
    }): AgentRegistryAgentSummary => ({
      id,
      agentKey: agent_key,
      version,
      lifecycleState: lifecycle_state,
      configurationFingerprint: configuration_fingerprint,
      ownerActorType: owner_actor_type,
      ownerActorId: owner_actor_id,
      createdAt: created_at,
    })
  );

const agentSchema = z
  .object({
    id: z.string(),
    agent_key: z.string(),
    version: z.number().int(),
    lifecycle_state: agentLifecycleStateSchema,
    configuration: agentConfigurationSchema,
    configuration_fingerprint: z.string(),
    owner_actor_type: registryOwnerActorTypeSchema,
    owner_actor_id: z.string(),
    created_at: z.string(),
  })
  .transform(
    ({
      id,
      agent_key,
      version,
      lifecycle_state,
      configuration,
      configuration_fingerprint,
      owner_actor_type,
      owner_actor_id,
      created_at,
    }): AgentRegistryAgent => ({
      id,
      agentKey: agent_key,
      version,
      lifecycleState: lifecycle_state,
      configuration,
      configurationFingerprint: configuration_fingerprint,
      ownerActorType: owner_actor_type,
      ownerActorId: owner_actor_id,
      createdAt: created_at,
    })
  );

const toolDefinitionSummarySchema = z
  .object({
    tool_key: z.string(),
    version: z.number().int(),
    lifecycle_state: z.literal('active'),
    definition_fingerprint: z.string(),
    schema_version: z.number().int(),
    display_name: z.string(),
    description: z.string(),
    tool_effect_class: toolEffectClassSchema,
    abstract_auth_scopes: z.array(z.string()),
    created_at: z.string(),
  })
  .transform(
    ({
      tool_key,
      version,
      lifecycle_state,
      definition_fingerprint,
      schema_version,
      display_name,
      description,
      tool_effect_class,
      abstract_auth_scopes,
      created_at,
    }): ToolRegistryToolDefinitionSummary => ({
      toolKey: tool_key,
      version,
      lifecycleState: lifecycle_state,
      definitionFingerprint: definition_fingerprint,
      schemaVersion: schema_version,
      displayName: display_name,
      description,
      toolEffectClass: tool_effect_class,
      abstractAuthScopes: abstract_auth_scopes,
      createdAt: created_at,
    })
  );

const toolDefinitionSchema = z
  .object({
    tool_key: z.string(),
    version: z.number().int(),
    lifecycle_state: toolDefinitionLifecycleStateSchema,
    definition_fingerprint: z.string(),
    schema_version: z.number().int(),
    display_name: z.string(),
    description: z.string(),
    tool_effect_class: toolEffectClassSchema,
    abstract_auth_scopes: z.array(z.string()),
    input_schema: jsonObjectSchema,
    output_schema: jsonObjectSchema,
    timeout_defaults: jsonObjectSchema,
    retry_contract: jsonObjectSchema,
    audit_contract: jsonObjectSchema,
    created_at: z.string(),
  })
  .transform(
    ({
      tool_key,
      version,
      lifecycle_state,
      definition_fingerprint,
      schema_version,
      display_name,
      description,
      tool_effect_class,
      abstract_auth_scopes,
      input_schema,
      output_schema,
      timeout_defaults,
      retry_contract,
      audit_contract,
      created_at,
    }): ToolRegistryToolDefinition => ({
      toolKey: tool_key,
      version,
      lifecycleState: lifecycle_state,
      definitionFingerprint: definition_fingerprint,
      schemaVersion: schema_version,
      displayName: display_name,
      description,
      toolEffectClass: tool_effect_class,
      abstractAuthScopes: abstract_auth_scopes,
      inputSchema: input_schema,
      outputSchema: output_schema,
      timeoutDefaults: timeout_defaults,
      retryContract: retry_contract,
      auditContract: audit_contract,
      createdAt: created_at,
    })
  );

const toolEnablementSchema = z
  .object({
    tool_key: z.string(),
    version: z.number().int(),
    lifecycle_state: toolEnablementLifecycleStateSchema,
    generation: z.number().int(),
    timeout_cap_ms: z.number().int().nullable(),
    approval_required: z.boolean(),
    allow_ttl_seconds: z.number().int().nullable(),
    audit_mode: toolEnablementAuditModeSchema.nullable(),
    updated_at: z.string(),
  })
  .transform(
    ({
      tool_key,
      version,
      lifecycle_state,
      generation,
      timeout_cap_ms,
      approval_required,
      allow_ttl_seconds,
      audit_mode,
      updated_at,
    }): ToolRegistryToolEnablement => ({
      toolKey: tool_key,
      version,
      lifecycleState: lifecycle_state,
      generation,
      timeoutCapMs: timeout_cap_ms,
      approvalRequired: approval_required,
      allowTtlSeconds: allow_ttl_seconds,
      auditMode: audit_mode,
      updatedAt: updated_at,
    })
  );

const connectorNormalizationContractSchema = z
  .object({
    evidence_family: z.string(),
    kernel_event_type: z.string(),
    kernel_event_schema_version: z.number().int(),
  })
  .transform(
    ({
      evidence_family,
      kernel_event_type,
      kernel_event_schema_version,
    }): ConnectorNormalizationContract => ({
      evidenceFamily: evidence_family,
      kernelEventType: kernel_event_type,
      kernelEventSchemaVersion: kernel_event_schema_version,
    })
  );

const connectorTypeSummarySchema = z
  .object({
    connector_key: z.string(),
    version: z.number().int(),
    lifecycle_state: connectorTypeLifecycleStateSchema,
    source_type: z.string(),
    connector_type_fingerprint: z.string(),
    capabilities: z.array(z.string()),
    created_at: z.string(),
  })
  .transform(
    ({
      connector_key,
      version,
      lifecycle_state,
      source_type,
      connector_type_fingerprint,
      capabilities,
      created_at,
    }): ConnectorRegistryTypeSummary => ({
      connectorKey: connector_key,
      version,
      lifecycleState: lifecycle_state,
      sourceType: source_type,
      connectorTypeFingerprint: connector_type_fingerprint,
      capabilities,
      createdAt: created_at,
    })
  );

const connectorTypeSchema = z
  .object({
    connector_key: z.string(),
    version: z.number().int(),
    lifecycle_state: connectorTypeLifecycleStateSchema,
    source_type: z.string(),
    connector_type_fingerprint: z.string(),
    capabilities: z.array(z.string()),
    normalization_contracts: z.array(connectorNormalizationContractSchema),
    delivery_behavior: jsonObjectSchema,
    created_at: z.string(),
  })
  .transform(
    ({
      connector_key,
      version,
      lifecycle_state,
      source_type,
      connector_type_fingerprint,
      capabilities,
      normalization_contracts,
      delivery_behavior,
      created_at,
    }): ConnectorRegistryType => ({
      connectorKey: connector_key,
      version,
      lifecycleState: lifecycle_state,
      sourceType: source_type,
      connectorTypeFingerprint: connector_type_fingerprint,
      capabilities,
      normalizationContracts: normalization_contracts,
      deliveryBehavior: delivery_behavior,
      createdAt: created_at,
    })
  );

const connectorRegistryProviderAccountSchema = z
  .object({ namespace: z.string(), external_account_ref: z.string() })
  .transform(
    ({ namespace, external_account_ref }): ConnectorRegistryProviderAccount => ({
      namespace,
      externalAccountRef: external_account_ref,
    })
  );

const connectorBindingSummarySchema = z
  .object({
    binding_key: z.string(),
    version: z.number().int(),
    lifecycle_state: connectorBindingLifecycleStateSchema,
    connector_type_key: z.string(),
    connector_type_version: z.number().int(),
    connector_type_fingerprint: z.string(),
    enabled_capabilities: z.array(z.string()),
    binding_fingerprint: z.string(),
    created_at: z.string(),
  })
  .transform(
    ({
      binding_key,
      version,
      lifecycle_state,
      connector_type_key,
      connector_type_version,
      connector_type_fingerprint,
      enabled_capabilities,
      binding_fingerprint,
      created_at,
    }): ConnectorRegistryBindingSummary => ({
      bindingKey: binding_key,
      version,
      lifecycleState: lifecycle_state,
      connectorTypeKey: connector_type_key,
      connectorTypeVersion: connector_type_version,
      connectorTypeFingerprint: connector_type_fingerprint,
      enabledCapabilities: enabled_capabilities,
      bindingFingerprint: binding_fingerprint,
      createdAt: created_at,
    })
  );

const connectorBindingSchema = z
  .object({
    binding_key: z.string(),
    version: z.number().int(),
    lifecycle_state: connectorBindingLifecycleStateSchema,
    connector_type_key: z.string(),
    connector_type_version: z.number().int(),
    connector_type_fingerprint: z.string(),
    provider_account: connectorRegistryProviderAccountSchema,
    config_ref: z.string(),
    credential_ref: z.string(),
    enabled_capabilities: z.array(z.string()),
    binding_fingerprint: z.string(),
    created_at: z.string(),
  })
  .transform(
    ({
      binding_key,
      version,
      lifecycle_state,
      connector_type_key,
      connector_type_version,
      connector_type_fingerprint,
      provider_account,
      config_ref,
      credential_ref,
      enabled_capabilities,
      binding_fingerprint,
      created_at,
    }): ConnectorRegistryBinding => ({
      bindingKey: binding_key,
      version,
      lifecycleState: lifecycle_state,
      connectorTypeKey: connector_type_key,
      connectorTypeVersion: connector_type_version,
      connectorTypeFingerprint: connector_type_fingerprint,
      providerAccount: provider_account,
      configRef: config_ref,
      credentialRef: credential_ref,
      enabledCapabilities: enabled_capabilities,
      bindingFingerprint: binding_fingerprint,
      createdAt: created_at,
    })
  );

function parseCursorRegistryPage<T>(
  response: unknown,
  itemSchema: z.ZodType<T>
): CursorRegistryPage<T> {
  return parseWithSchema(
    unwrapLoggedCoreResult(response),
    z
      .object({ items: z.array(itemSchema), next_cursor: z.string().nullable() })
      .transform(
        ({ items, next_cursor }): CursorRegistryPage<T> => ({ items, nextCursor: next_cursor })
      )
  );
}

function parseUnpagedRegistryCollection<T>(
  response: unknown,
  itemSchema: z.ZodType<T>
): UnpagedRegistryCollection<T> {
  return parseWithSchema(
    unwrapLoggedCoreResult(response),
    z
      .object({ items: z.array(itemSchema) })
      .transform(({ items }): UnpagedRegistryCollection<T> => ({ items }))
  );
}

function unwrapLoggedCoreResult(response: unknown): unknown {
  if (
    response &&
    typeof response === 'object' &&
    !Array.isArray(response) &&
    'result' in response &&
    'logs' in response &&
    Array.isArray((response as { logs?: unknown }).logs)
  ) {
    return (response as { result: unknown }).result;
  }
  return response;
}

function normalizeRegistrySchemaError(error: z.ZodError): never {
  throw new CoreRpcError(
    `Registry bridge response shape mismatch: ${error.issues[0]?.message ?? 'invalid payload'}`,
    'unknown',
    undefined,
    { kind: 'YouPetCoreResponseShape' }
  );
}

function parseWithSchema<T>(response: unknown, schema: z.ZodType<T>): T {
  try {
    return schema.parse(response);
  } catch (error) {
    if (error instanceof z.ZodError) {
      normalizeRegistrySchemaError(error);
    }
    throw error;
  }
}

function unwrapExactRecord(response: unknown, recordKey: string): unknown {
  response = unwrapLoggedCoreResult(response);
  if (response && typeof response === 'object' && recordKey in response) {
    return (response as Record<string, unknown>)[recordKey];
  }
  return response;
}

function pruneParams(params: Record<string, unknown>): Record<string, unknown> {
  const next: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined) {
      next[key] = value;
    }
  }
  return next;
}

function assertValidRegistryKey(field: string, key: string): string {
  const trimmed = key.trim();
  if (!trimmed) {
    throw new Error(`${field} must be non-empty`);
  }
  if (trimmed.length > MAX_REGISTRY_KEY_LENGTH) {
    throw new Error(`${field} must be at most ${MAX_REGISTRY_KEY_LENGTH} characters`);
  }
  return trimmed;
}

function assertPositiveVersion(version: number): number {
  if (!Number.isInteger(version) || version < 1) {
    throw new Error('version must be a positive integer');
  }
  return version;
}

function buildCursorParams(params?: RegistryCursorListParams): Record<string, unknown> {
  if (!params) {
    return {};
  }

  const next = pruneParams({ limit: params.limit, cursor: params.cursor?.trim() || undefined });

  if ('limit' in next) {
    const limit = next.limit;
    if (!Number.isInteger(limit) || Number(limit) < 1 || Number(limit) > 200) {
      throw new Error('limit must be an integer between 1 and 200');
    }
  }

  return next;
}

function buildExactParams(
  field: 'agentKey' | 'toolKey' | 'connectorKey' | 'bindingKey',
  key: string,
  version: number
): { version: number } & Record<typeof field, string> {
  return {
    [field]: assertValidRegistryKey(field, key),
    version: assertPositiveVersion(version),
  } as { version: number } & Record<typeof field, string>;
}

function parseRegistryBridgeErrorKind(kind: unknown): RegistryBridgeErrorKind | null {
  return REGISTRY_BRIDGE_ERROR_KINDS.includes(kind as RegistryBridgeErrorKind)
    ? (kind as RegistryBridgeErrorKind)
    : null;
}

export function extractRegistryBridgeErrorMeta(error: unknown): RegistryBridgeErrorMeta | null {
  if (!(error instanceof CoreRpcError)) {
    return null;
  }

  const data = error.data;
  if (!data || typeof data !== 'object') {
    return null;
  }

  const kind = parseRegistryBridgeErrorKind((data as { kind?: unknown }).kind);
  if (!kind) {
    return null;
  }

  const meta: RegistryBridgeErrorMeta = { kind };
  const youpet = (data as { youpet?: unknown }).youpet;
  if (youpet && typeof youpet === 'object') {
    const record = youpet as {
      http_status?: unknown;
      code?: unknown;
      retry_after_seconds?: unknown;
    };
    if (typeof record.http_status === 'number') {
      meta.httpStatus = record.http_status;
    }
    if (typeof record.code === 'string') {
      meta.coreCode = record.code;
    }
    if (typeof record.retry_after_seconds === 'number') {
      meta.retryAfterSeconds = record.retry_after_seconds;
    }
  }

  return meta;
}

async function getExactRecord<T>(
  method: (typeof CORE_RPC_METHODS)[keyof typeof CORE_RPC_METHODS],
  params: Record<string, unknown>,
  recordKey: string,
  recordSchema: z.ZodType<T>
): Promise<T> {
  const response = await callCoreRpc<unknown>({ method, params });
  return parseWithSchema(unwrapExactRecord(response, recordKey), recordSchema);
}

export const coreRegistriesClient = {
  listAgents: async (
    params?: RegistryCursorListParams
  ): Promise<CursorRegistryPage<AgentRegistryAgentSummary>> =>
    parseCursorRegistryPage(
      await callCoreRpc<unknown>({
        method: CORE_RPC_METHODS.youpetRegistryListAgents,
        params: buildCursorParams(params),
      }),
      agentSummarySchema
    ),

  getAgentVersion: (params: { agentKey: string; version: number }): Promise<AgentRegistryAgent> =>
    getExactRecord(
      CORE_RPC_METHODS.youpetRegistryGetAgentVersion,
      buildExactParams('agentKey', params.agentKey, params.version),
      'agent',
      agentSchema
    ),

  listToolDefinitions: async (
    params?: RegistryCursorListParams
  ): Promise<CursorRegistryPage<ToolRegistryToolDefinitionSummary>> =>
    parseCursorRegistryPage(
      await callCoreRpc<unknown>({
        method: CORE_RPC_METHODS.youpetRegistryListToolDefinitions,
        params: buildCursorParams(params),
      }),
      toolDefinitionSummarySchema
    ),

  getToolDefinitionVersion: (params: {
    toolKey: string;
    version: number;
  }): Promise<ToolRegistryToolDefinition> =>
    getExactRecord(
      CORE_RPC_METHODS.youpetRegistryGetToolDefinitionVersion,
      buildExactParams('toolKey', params.toolKey, params.version),
      'toolDefinition',
      toolDefinitionSchema
    ),

  listToolEnablements: async (): Promise<UnpagedRegistryCollection<ToolRegistryToolEnablement>> =>
    parseUnpagedRegistryCollection(
      await callCoreRpc<unknown>({
        method: CORE_RPC_METHODS.youpetRegistryListToolEnablements,
        params: {},
      }),
      toolEnablementSchema
    ),

  getToolEnablementVersion: (params: {
    toolKey: string;
    version: number;
  }): Promise<ToolRegistryToolEnablement> =>
    getExactRecord(
      CORE_RPC_METHODS.youpetRegistryGetToolEnablementVersion,
      buildExactParams('toolKey', params.toolKey, params.version),
      'toolEnablement',
      toolEnablementSchema
    ),

  listConnectorTypes: async (
    params?: RegistryCursorListParams
  ): Promise<CursorRegistryPage<ConnectorRegistryTypeSummary>> =>
    parseCursorRegistryPage(
      await callCoreRpc<unknown>({
        method: CORE_RPC_METHODS.youpetRegistryListConnectorTypes,
        params: buildCursorParams(params),
      }),
      connectorTypeSummarySchema
    ),

  getConnectorTypeVersion: (params: {
    connectorKey: string;
    version: number;
  }): Promise<ConnectorRegistryType> =>
    getExactRecord(
      CORE_RPC_METHODS.youpetRegistryGetConnectorTypeVersion,
      buildExactParams('connectorKey', params.connectorKey, params.version),
      'connectorType',
      connectorTypeSchema
    ),

  listConnectorBindings: async (
    params?: RegistryCursorListParams
  ): Promise<CursorRegistryPage<ConnectorRegistryBindingSummary>> =>
    parseCursorRegistryPage(
      await callCoreRpc<unknown>({
        method: CORE_RPC_METHODS.youpetRegistryListConnectorBindings,
        params: buildCursorParams(params),
      }),
      connectorBindingSummarySchema
    ),

  getConnectorBindingVersion: (params: {
    bindingKey: string;
    version: number;
  }): Promise<ConnectorRegistryBinding> =>
    getExactRecord(
      CORE_RPC_METHODS.youpetRegistryGetConnectorBindingVersion,
      buildExactParams('bindingKey', params.bindingKey, params.version),
      'connectorBinding',
      connectorBindingSchema
    ),
};
