export const CORE_RPC_METHODS = {
  configGet: 'openhuman.config_get',
  configGetAnalyticsSettings: 'openhuman.config_get_analytics_settings',
  configGetComposioTriggerSettings: 'openhuman.config_get_composio_trigger_settings',
  configGetRuntimeFlags: 'openhuman.config_get_runtime_flags',
  configSetBrowserAllowAll: 'openhuman.config_set_browser_allow_all',
  configUpdateAnalyticsSettings: 'openhuman.config_update_analytics_settings',
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
} as const;

export type CoreRpcMethod = (typeof CORE_RPC_METHODS)[keyof typeof CORE_RPC_METHODS];

export const LEGACY_METHOD_ALIASES: Record<string, CoreRpcMethod> = {
  'openhuman.get_analytics_settings': CORE_RPC_METHODS.configGetAnalyticsSettings,
  'openhuman.get_composio_trigger_settings': CORE_RPC_METHODS.configGetComposioTriggerSettings,
  'openhuman.get_config': CORE_RPC_METHODS.configGet,
  'openhuman.get_runtime_flags': CORE_RPC_METHODS.configGetRuntimeFlags,
  'openhuman.ping': CORE_RPC_METHODS.corePing,
  'openhuman.set_browser_allow_all': CORE_RPC_METHODS.configSetBrowserAllowAll,
  'openhuman.update_analytics_settings': CORE_RPC_METHODS.configUpdateAnalyticsSettings,
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
