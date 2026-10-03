export const CORE_RPC_METHODS = {
  authClearCredential: 'openhuman.auth_clear_credential',
  authGetSessionToken: 'openhuman.auth_get_session_token',
  authGetState: 'openhuman.auth_get_state',
  authSetCredential: 'openhuman.auth_set_credential',
  configGet: 'openhuman.config_get',
  configGetAgentPaths: 'openhuman.config_get_agent_paths',
  configGetAgentSettings: 'openhuman.config_get_agent_settings',
  configGetAnalyticsSettings: 'openhuman.config_get_analytics_settings',
  configGetAutonomySettings: 'openhuman.config_get_autonomy_settings',
  configGetComposioTriggerSettings: 'openhuman.config_get_composio_trigger_settings',
  configGetDashboardSettings: 'openhuman.config_get_dashboard_settings',
  configGetRuntimeFlags: 'openhuman.config_get_runtime_flags',
  configGetPrivacyMode: 'openhuman.config_get_privacy_mode',
  configGetSandboxSettings: 'openhuman.config_get_sandbox_settings',
  configGetSearchSettings: 'openhuman.config_get_search_settings',
  configSetPrivacyMode: 'openhuman.config_set_privacy_mode',
  configUpdateSearchSettings: 'openhuman.config_update_search_settings',
  configSetBrowserAllowAll: 'openhuman.config_set_browser_allow_all',
  configUpdateAgentPaths: 'openhuman.config_update_agent_paths',
  configUpdateAgentSettings: 'openhuman.config_update_agent_settings',
  configUpdateAnalyticsSettings: 'openhuman.config_update_analytics_settings',
  configUpdateAutonomySettings: 'openhuman.config_update_autonomy_settings',
  configUpdateBrowserSettings: 'openhuman.config_update_browser_settings',
  configUpdateComputerSettings: 'openhuman.config_update_computer_settings',
  configUpdateComposioTriggerSettings: 'openhuman.config_update_composio_trigger_settings',
  configUpdateLocalAiSettings: 'openhuman.config_update_local_ai_settings',
  configUpdateMemorySettings: 'openhuman.config_update_memory_settings',
  configUpdateModelSettings: 'openhuman.config_update_model_settings',
  configUpdateRuntimeSettings: 'openhuman.config_update_runtime_settings',
  configUpdateSandboxSettings: 'openhuman.config_update_sandbox_settings',
  configWorkspaceOnboardingFlagExists: 'openhuman.config_workspace_onboarding_flag_exists',
  configWorkspaceOnboardingFlagSet: 'openhuman.config_workspace_onboarding_flag_set',
  corePing: 'core.ping',
  // Memory v2 (docs/specs/memory-v2.md), wrapped by services/api/memoryApi.ts.
  memoryEnginesList: 'openhuman.memory_engines_list',
  memoryEngineGet: 'openhuman.memory_engine_get',
  memoryEngineSet: 'openhuman.memory_engine_set',
  memoryRecall: 'openhuman.memory_recall',
  memoryFetch: 'openhuman.memory_fetch',
  memoryLearn: 'openhuman.memory_learn',
  memoryForget: 'openhuman.memory_forget',
  memoryItemsList: 'openhuman.memory_items_list',
  memoryConversationsGet: 'openhuman.memory_conversations_get',
  memoryConversationsSet: 'openhuman.memory_conversations_set',
  memorySourcesList: 'openhuman.memory_sources_list',
  memorySourcesAdd: 'openhuman.memory_sources_add',
  memorySourcesRemove: 'openhuman.memory_sources_remove',
  memorySourcesSync: 'openhuman.memory_sources_sync',
  memoryContextGet: 'openhuman.memory_context_get',
  memoryContextRefresh: 'openhuman.memory_context_refresh',
  memoryContextSet: 'openhuman.memory_context_set',
  memoryImportScan: 'openhuman.memory_import_scan',
  memoryImportStart: 'openhuman.memory_import_start',
  memoryImportStatus: 'openhuman.memory_import_status',
  inferenceAgentChat: 'openhuman.inference_agent_chat',
  inferenceAgentChatSimple: 'openhuman.inference_agent_chat_simple',
  inferenceDiagnostics: 'openhuman.inference_diagnostics',
  inferenceGetClientConfig: 'openhuman.inference_get_client_config',
  inferenceListModels: 'openhuman.inference_list_models',
  inferenceTestConnection: 'openhuman.inference_test_connection',
  inferenceTranscribe: 'openhuman.inference_transcribe',
  inferenceTranscribeBytes: 'openhuman.inference_transcribe_bytes',
  inferenceTts: 'openhuman.inference_tts',
  inferenceUpdateLocalSettings: 'openhuman.inference_update_local_settings',
  inferenceUpdateModelSettings: 'openhuman.inference_update_model_settings',
  providersListModels: 'openhuman.inference_list_models',
  embeddingsGetSettings: 'openhuman.embeddings_get_settings',
  embeddingsUpdateSettings: 'openhuman.embeddings_update_settings',
  embeddingsSetApiKey: 'openhuman.embeddings_set_api_key',
  embeddingsClearApiKey: 'openhuman.embeddings_clear_api_key',
  embeddingsEmbed: 'openhuman.embeddings_embed',
  embeddingsTestConnection: 'openhuman.embeddings_test_connection',
  channelsList: 'openhuman.channels_list',
  mcpClientsInstalledList: 'openhuman.mcp_clients_installed_list',
  mcpClientsToolCall: 'openhuman.mcp_clients_tool_call',
  toolRegistryDiagnostics: 'openhuman.tool_registry_diagnostics',
  healthSnapshot: 'openhuman.health_snapshot',
  healthSystemInfo: 'openhuman.health_system_info',
} as const;

type CoreRpcMethod = (typeof CORE_RPC_METHODS)[keyof typeof CORE_RPC_METHODS];

export const LEGACY_METHOD_ALIASES: Record<string, CoreRpcMethod> = {
  // The session RPCs became the kind-agnostic credential pair once the core
  // stopped validating sessions itself. Mirrored in
  // crates/openhuman-core/src/core/legacy_aliases.rs.
  'openhuman.auth_clear_session': CORE_RPC_METHODS.authClearCredential,
  'openhuman.auth_store_session': CORE_RPC_METHODS.authSetCredential,
  // #3565: old desktop clients used dotted namespace/function channel calls.
  'channels.list': CORE_RPC_METHODS.channelsList,
  // MCP clients — old method names that appeared in Sentry (CORE-RUST-DR/DS/DT/DV/DW).
  // See crates/openhuman-core/src/core/legacy_aliases.rs for the Rust-side mirror of this table.
  'mcp_clients.list': CORE_RPC_METHODS.mcpClientsInstalledList,
  'openhuman.channels.list': CORE_RPC_METHODS.channelsList,
  'openhuman.mcp_clients_list': CORE_RPC_METHODS.mcpClientsInstalledList,
  'openhuman.mcp_list': CORE_RPC_METHODS.mcpClientsInstalledList,
  'openhuman.mcp_servers_list': CORE_RPC_METHODS.mcpClientsInstalledList,
  'openhuman.tool_registry_call': CORE_RPC_METHODS.mcpClientsToolCall,
  // #3294: old desktop bundles called the tool-registry diagnostics
  // controller with the dotted `tool_registry.diagnostics` spelling before the
  // canonical `openhuman.tool_registry_diagnostics` form, so the Tool Policy
  // diagnostics panel failed with "unknown method". Keep in sync with the
  // Rust-side mirror in crates/openhuman-core/src/core/legacy_aliases.rs.
  'tool_registry.diagnostics': CORE_RPC_METHODS.toolRegistryDiagnostics,
  'openhuman.get_analytics_settings': CORE_RPC_METHODS.configGetAnalyticsSettings,
  'openhuman.get_composio_trigger_settings': CORE_RPC_METHODS.configGetComposioTriggerSettings,
  'openhuman.get_dashboard_settings': CORE_RPC_METHODS.configGetDashboardSettings,
  'openhuman.get_config': CORE_RPC_METHODS.configGet,
  'openhuman.get_runtime_flags': CORE_RPC_METHODS.configGetRuntimeFlags,
  'openhuman.ping': CORE_RPC_METHODS.corePing,
  'openhuman.set_browser_allow_all': CORE_RPC_METHODS.configSetBrowserAllowAll,
  'openhuman.update_analytics_settings': CORE_RPC_METHODS.configUpdateAnalyticsSettings,
  'openhuman.update_autonomy_settings': CORE_RPC_METHODS.configUpdateAutonomySettings,
  'openhuman.update_browser_settings': CORE_RPC_METHODS.configUpdateBrowserSettings,
  'openhuman.update_composio_trigger_settings':
    CORE_RPC_METHODS.configUpdateComposioTriggerSettings,
  'openhuman.update_local_ai_settings': CORE_RPC_METHODS.inferenceUpdateLocalSettings,
  'openhuman.update_memory_settings': CORE_RPC_METHODS.configUpdateMemorySettings,
  'openhuman.update_model_settings': CORE_RPC_METHODS.inferenceUpdateModelSettings,
  'openhuman.update_runtime_settings': CORE_RPC_METHODS.configUpdateRuntimeSettings,
  'openhuman.workspace_onboarding_flag_exists':
    CORE_RPC_METHODS.configWorkspaceOnboardingFlagExists,
  'openhuman.workspace_onboarding_flag_set': CORE_RPC_METHODS.configWorkspaceOnboardingFlagSet,
  'openhuman.local_ai_agent_chat': CORE_RPC_METHODS.inferenceAgentChat,
  'openhuman.local_ai_agent_chat_simple': CORE_RPC_METHODS.inferenceAgentChatSimple,
  'openhuman.local_ai_diagnostics': CORE_RPC_METHODS.inferenceDiagnostics,
  'openhuman.local_ai_test_connection': CORE_RPC_METHODS.inferenceTestConnection,
  'openhuman.local_ai_transcribe': CORE_RPC_METHODS.inferenceTranscribe,
  'openhuman.local_ai_transcribe_bytes': CORE_RPC_METHODS.inferenceTranscribeBytes,
  'openhuman.local_ai_tts': CORE_RPC_METHODS.inferenceTts,
  'openhuman.providers_list_models': CORE_RPC_METHODS.inferenceListModels,
  health_snapshot: CORE_RPC_METHODS.healthSnapshot,
  // Dotted / bare health probes from older clients and SDK callers (#3566,
  // Sentry CORE-2C). No distinct status/get handler exists — the snapshot
  // already carries the health verdict — so all four alias to the snapshot.
  // Keep in sync with crates/openhuman-core/src/core/legacy_aliases.rs (drift guard enforces it).
  health: CORE_RPC_METHODS.healthSnapshot,
  'health.get': CORE_RPC_METHODS.healthSnapshot,
  'health.snapshot': CORE_RPC_METHODS.healthSnapshot,
  'health.status': CORE_RPC_METHODS.healthSnapshot,
  // `openhuman.system_info` was used by older clients / SDK callers before the
  // method was namespaced as `openhuman.health_system_info`.
  // Sentry CORE-RUST-G0 — https://sentry.tinyhumans.ai/organizations/tinyhumans/issues/6340/
  'openhuman.system_info': CORE_RPC_METHODS.healthSystemInfo,
};

export function normalizeRpcMethod(method: string): string {
  const normalized = method.trim().toLowerCase();

  if (normalized in LEGACY_METHOD_ALIASES) {
    return LEGACY_METHOD_ALIASES[normalized];
  }

  if (normalized.startsWith('openhuman.auth.')) {
    return `openhuman.auth_${normalized.slice('openhuman.auth.'.length).split('.').join('_')}`;
  }

  return normalized;
}
