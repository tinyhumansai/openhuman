export const CORE_RPC_METHODS = {
  configGet: 'openhuman.config_get',
  configGetRuntimeFlags: 'openhuman.config_get_runtime_flags',
  configSetBrowserAllowAll: 'openhuman.config_set_browser_allow_all',
  configUpdateBrowserSettings: 'openhuman.config_update_browser_settings',
  configUpdateMemorySettings: 'openhuman.config_update_memory_settings',
  configUpdateModelSettings: 'openhuman.config_update_model_settings',
  configUpdateRuntimeSettings: 'openhuman.config_update_runtime_settings',
  configUpdateScreenIntelligenceSettings: 'openhuman.config_update_screen_intelligence_settings',
  configWorkspaceOnboardingFlagExists: 'openhuman.config_workspace_onboarding_flag_exists',
  configWorkspaceOnboardingFlagSet: 'openhuman.config_workspace_onboarding_flag_set',
  screenIntelligenceStatus: 'openhuman.screen_intelligence_status',
} as const;

export type CoreRpcMethod = (typeof CORE_RPC_METHODS)[keyof typeof CORE_RPC_METHODS];

export const LEGACY_METHOD_ALIASES: Record<string, CoreRpcMethod> = {
  'openhuman.get_config': CORE_RPC_METHODS.configGet,
  'openhuman.get_runtime_flags': CORE_RPC_METHODS.configGetRuntimeFlags,
  'openhuman.set_browser_allow_all': CORE_RPC_METHODS.configSetBrowserAllowAll,
  'openhuman.update_browser_settings': CORE_RPC_METHODS.configUpdateBrowserSettings,
  'openhuman.update_memory_settings': CORE_RPC_METHODS.configUpdateMemorySettings,
  'openhuman.update_model_settings': CORE_RPC_METHODS.configUpdateModelSettings,
  'openhuman.update_runtime_settings': CORE_RPC_METHODS.configUpdateRuntimeSettings,
  'openhuman.update_screen_intelligence_settings':
    CORE_RPC_METHODS.configUpdateScreenIntelligenceSettings,
  'openhuman.workspace_onboarding_flag_exists':
    CORE_RPC_METHODS.configWorkspaceOnboardingFlagExists,
  'openhuman.workspace_onboarding_flag_set': CORE_RPC_METHODS.configWorkspaceOnboardingFlagSet,
};

export function normalizeRpcMethod(method: string): string {
  if (method in LEGACY_METHOD_ALIASES) {
    return LEGACY_METHOD_ALIASES[method];
  }

  if (method.startsWith('openhuman.auth.')) {
    return `openhuman.auth_${method.slice('openhuman.auth.'.length).split('.').join('_')}`;
  }

  if (method.startsWith('openhuman.accessibility_')) {
    return method.replace('openhuman.accessibility_', 'openhuman.screen_intelligence_');
  }

  return method;
}
