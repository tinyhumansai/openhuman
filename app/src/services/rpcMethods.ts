export const CORE_RPC_METHODS = {
  configGet: 'openhuman.config_get',
  configGetAnalyticsSettings: 'openhuman.config_get_analytics_settings',
  configGetAutonomySettings: 'openhuman.config_get_autonomy_settings',
  configGetComposioTriggerSettings: 'openhuman.config_get_composio_trigger_settings',
  configGetDashboardSettings: 'openhuman.config_get_dashboard_settings',
  configGetRuntimeFlags: 'openhuman.config_get_runtime_flags',
  configGetSearchSettings: 'openhuman.config_get_search_settings',
  configUpdateSearchSettings: 'openhuman.config_update_search_settings',
  configSetBrowserAllowAll: 'openhuman.config_set_browser_allow_all',
  configUpdateAnalyticsSettings: 'openhuman.config_update_analytics_settings',
  configUpdateAutonomySettings: 'openhuman.config_update_autonomy_settings',
  configUpdateBrowserSettings: 'openhuman.config_update_browser_settings',
  configUpdateComposioTriggerSettings: 'openhuman.config_update_composio_trigger_settings',
  configUpdateLocalAiSettings: 'openhuman.config_update_local_ai_settings',
  configUpdateMemorySettings: 'openhuman.config_update_memory_settings',
  configUpdateModelSettings: 'openhuman.config_update_model_settings',
  configUpdateRuntimeSettings: 'openhuman.config_update_runtime_settings',
  configUpdateScreenIntelligenceSettings: 'openhuman.config_update_screen_intelligence_settings',
  configWorkspaceOnboardingFlagExists: 'openhuman.config_workspace_onboarding_flag_exists',
  configWorkspaceOnboardingFlagSet: 'openhuman.config_workspace_onboarding_flag_set',
  corePing: 'core.ping',
  inferenceApplyPreset: 'openhuman.inference_apply_preset',
  inferenceDiagnostics: 'openhuman.inference_diagnostics',
  inferenceDeviceProfile: 'openhuman.inference_device_profile',
  inferenceGetClientConfig: 'openhuman.inference_get_client_config',
  inferenceListModels: 'openhuman.inference_list_models',
  inferencePresets: 'openhuman.inference_presets',
  inferenceUpdateLocalSettings: 'openhuman.inference_update_local_settings',
  inferenceUpdateModelSettings: 'openhuman.inference_update_model_settings',
  providersListModels: 'openhuman.inference_list_models',
  screenIntelligenceStatus: 'openhuman.screen_intelligence_status',
  embeddingsGetSettings: 'openhuman.embeddings_get_settings',
  embeddingsUpdateSettings: 'openhuman.embeddings_update_settings',
  embeddingsSetApiKey: 'openhuman.embeddings_set_api_key',
  embeddingsClearApiKey: 'openhuman.embeddings_clear_api_key',
  embeddingsEmbed: 'openhuman.embeddings_embed',
  embeddingsTestConnection: 'openhuman.embeddings_test_connection',
  mcpClientsInstalledList: 'openhuman.mcp_clients_installed_list',
  mcpClientsToolCall: 'openhuman.mcp_clients_tool_call',
  healthSnapshot: 'openhuman.health_snapshot',
  healthSystemInfo: 'openhuman.health_system_info',
} as const;

export type CoreRpcMethod = (typeof CORE_RPC_METHODS)[keyof typeof CORE_RPC_METHODS];

export const LEGACY_METHOD_ALIASES: Record<string, CoreRpcMethod> = {
  // MCP clients — old method names that appeared in Sentry (CORE-RUST-DR/DS/DT/DV/DW).
  // See src/core/legacy_aliases.rs for the Rust-side mirror of this table.
  'mcp_clients.list': CORE_RPC_METHODS.mcpClientsInstalledList,
  'openhuman.mcp_clients_list': CORE_RPC_METHODS.mcpClientsInstalledList,
  'openhuman.mcp_list': CORE_RPC_METHODS.mcpClientsInstalledList,
  'openhuman.mcp_servers_list': CORE_RPC_METHODS.mcpClientsInstalledList,
  'openhuman.tool_registry_call': CORE_RPC_METHODS.mcpClientsToolCall,
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
  'openhuman.update_screen_intelligence_settings':
    CORE_RPC_METHODS.configUpdateScreenIntelligenceSettings,
  'openhuman.workspace_onboarding_flag_exists':
    CORE_RPC_METHODS.configWorkspaceOnboardingFlagExists,
  'openhuman.workspace_onboarding_flag_set': CORE_RPC_METHODS.configWorkspaceOnboardingFlagSet,
  'openhuman.local_ai_apply_preset': CORE_RPC_METHODS.inferenceApplyPreset,
  'openhuman.local_ai_device_profile': CORE_RPC_METHODS.inferenceDeviceProfile,
  'openhuman.local_ai_diagnostics': CORE_RPC_METHODS.inferenceDiagnostics,
  'openhuman.local_ai_presets': CORE_RPC_METHODS.inferencePresets,
  'openhuman.providers_list_models': CORE_RPC_METHODS.inferenceListModels,
  'openhuman.inference_embed': CORE_RPC_METHODS.embeddingsEmbed,
  health_snapshot: CORE_RPC_METHODS.healthSnapshot,
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

  if (normalized.startsWith('openhuman.accessibility_')) {
    return normalized.replace('openhuman.accessibility_', 'openhuman.screen_intelligence_');
  }

  return normalized;
}
