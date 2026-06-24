export const CORE_RPC_METHODS = {
  configGet: 'openhuman.config_get',
  configGetAgentPaths: 'openhuman.config_get_agent_paths',
  configGetAgentSettings: 'openhuman.config_get_agent_settings',
  configGetAnalyticsSettings: 'openhuman.config_get_analytics_settings',
  configGetAutonomySettings: 'openhuman.config_get_autonomy_settings',
  configGetComposioTriggerSettings: 'openhuman.config_get_composio_trigger_settings',
  configGetDashboardSettings: 'openhuman.config_get_dashboard_settings',
  configGetRuntimeFlags: 'openhuman.config_get_runtime_flags',
  configGetMemorySyncSettings: 'openhuman.config_get_memory_sync_settings',
  configGetSandboxSettings: 'openhuman.config_get_sandbox_settings',
  configGetSearchSettings: 'openhuman.config_get_search_settings',
  configUpdateSearchSettings: 'openhuman.config_update_search_settings',
  configSetBrowserAllowAll: 'openhuman.config_set_browser_allow_all',
  configUpdateAgentPaths: 'openhuman.config_update_agent_paths',
  configUpdateAgentSettings: 'openhuman.config_update_agent_settings',
  configUpdateAnalyticsSettings: 'openhuman.config_update_analytics_settings',
  configUpdateAutonomySettings: 'openhuman.config_update_autonomy_settings',
  configUpdateBrowserSettings: 'openhuman.config_update_browser_settings',
  configUpdateComposioTriggerSettings: 'openhuman.config_update_composio_trigger_settings',
  configUpdateLocalAiSettings: 'openhuman.config_update_local_ai_settings',
  configUpdateMemorySettings: 'openhuman.config_update_memory_settings',
  configUpdateMemorySyncSettings: 'openhuman.config_update_memory_sync_settings',
  configUpdateModelSettings: 'openhuman.config_update_model_settings',
  configUpdateRuntimeSettings: 'openhuman.config_update_runtime_settings',
  configUpdateSandboxSettings: 'openhuman.config_update_sandbox_settings',
  configUpdateScreenIntelligenceSettings: 'openhuman.config_update_screen_intelligence_settings',
  configWorkspaceOnboardingFlagExists: 'openhuman.config_workspace_onboarding_flag_exists',
  configWorkspaceOnboardingFlagSet: 'openhuman.config_workspace_onboarding_flag_set',
  corePing: 'core.ping',
  inferenceAgentChat: 'openhuman.inference_agent_chat',
  inferenceAgentChatSimple: 'openhuman.inference_agent_chat_simple',
  inferenceApplyPreset: 'openhuman.inference_apply_preset',
  inferenceAssetsStatus: 'openhuman.inference_assets_status',
  inferenceDiagnostics: 'openhuman.inference_diagnostics',
  inferenceDeviceProfile: 'openhuman.inference_device_profile',
  inferenceDownloadAsset: 'openhuman.inference_download_asset',
  inferenceDownloadsProgress: 'openhuman.inference_downloads_progress',
  inferenceGetClientConfig: 'openhuman.inference_get_client_config',
  inferenceInstallPiper: 'openhuman.inference_install_piper',
  inferenceInstallWhisper: 'openhuman.inference_install_whisper',
  inferenceListModels: 'openhuman.inference_list_models',
  inferencePiperInstallStatus: 'openhuman.inference_piper_install_status',
  inferencePresets: 'openhuman.inference_presets',
  inferenceTestConnection: 'openhuman.inference_test_connection',
  inferenceTranscribe: 'openhuman.inference_transcribe',
  inferenceTranscribeBytes: 'openhuman.inference_transcribe_bytes',
  inferenceTts: 'openhuman.inference_tts',
  inferenceUpdateLocalSettings: 'openhuman.inference_update_local_settings',
  inferenceUpdateModelSettings: 'openhuman.inference_update_model_settings',
  inferenceWhisperInstallStatus: 'openhuman.inference_whisper_install_status',
  providersListModels: 'openhuman.inference_list_models',
  screenIntelligenceStatus: 'openhuman.screen_intelligence_status',
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

export type CoreRpcMethod = (typeof CORE_RPC_METHODS)[keyof typeof CORE_RPC_METHODS];

export const LEGACY_METHOD_ALIASES: Record<string, CoreRpcMethod> = {
  // #3565: old desktop clients used dotted namespace/function channel calls.
  'channels.list': CORE_RPC_METHODS.channelsList,
  // MCP clients — old method names that appeared in Sentry (CORE-RUST-DR/DS/DT/DV/DW).
  // See src/core/legacy_aliases.rs for the Rust-side mirror of this table.
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
  // Rust-side mirror in src/core/legacy_aliases.rs.
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
  'openhuman.update_screen_intelligence_settings':
    CORE_RPC_METHODS.configUpdateScreenIntelligenceSettings,
  'openhuman.workspace_onboarding_flag_exists':
    CORE_RPC_METHODS.configWorkspaceOnboardingFlagExists,
  'openhuman.workspace_onboarding_flag_set': CORE_RPC_METHODS.configWorkspaceOnboardingFlagSet,
  'openhuman.local_ai_agent_chat': CORE_RPC_METHODS.inferenceAgentChat,
  'openhuman.local_ai_agent_chat_simple': CORE_RPC_METHODS.inferenceAgentChatSimple,
  'openhuman.local_ai_apply_preset': CORE_RPC_METHODS.inferenceApplyPreset,
  'openhuman.local_ai_assets_status': CORE_RPC_METHODS.inferenceAssetsStatus,
  'openhuman.local_ai_device_profile': CORE_RPC_METHODS.inferenceDeviceProfile,
  'openhuman.local_ai_diagnostics': CORE_RPC_METHODS.inferenceDiagnostics,
  'openhuman.local_ai_download_asset': CORE_RPC_METHODS.inferenceDownloadAsset,
  'openhuman.local_ai_downloads_progress': CORE_RPC_METHODS.inferenceDownloadsProgress,
  'openhuman.local_ai_install_piper': CORE_RPC_METHODS.inferenceInstallPiper,
  'openhuman.local_ai_install_whisper': CORE_RPC_METHODS.inferenceInstallWhisper,
  'openhuman.local_ai_piper_install_status': CORE_RPC_METHODS.inferencePiperInstallStatus,
  'openhuman.local_ai_presets': CORE_RPC_METHODS.inferencePresets,
  'openhuman.local_ai_test_connection': CORE_RPC_METHODS.inferenceTestConnection,
  'openhuman.local_ai_transcribe': CORE_RPC_METHODS.inferenceTranscribe,
  'openhuman.local_ai_transcribe_bytes': CORE_RPC_METHODS.inferenceTranscribeBytes,
  'openhuman.local_ai_tts': CORE_RPC_METHODS.inferenceTts,
  'openhuman.local_ai_whisper_install_status': CORE_RPC_METHODS.inferenceWhisperInstallStatus,
  'openhuman.providers_list_models': CORE_RPC_METHODS.inferenceListModels,
  'openhuman.inference_embed': CORE_RPC_METHODS.embeddingsEmbed,
  health_snapshot: CORE_RPC_METHODS.healthSnapshot,
  // Dotted / bare health probes from older clients and SDK callers (#3566,
  // Sentry CORE-2C). No distinct status/get handler exists — the snapshot
  // already carries the health verdict — so all four alias to the snapshot.
  // Keep in sync with src/core/legacy_aliases.rs (drift guard enforces it).
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

  if (normalized.startsWith('openhuman.accessibility_')) {
    return normalized.replace('openhuman.accessibility_', 'openhuman.screen_intelligence_');
  }

  return normalized;
}
