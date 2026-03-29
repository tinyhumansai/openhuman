import {
  ChatBubbleLeftRightIcon,
  CogIcon,
  CpuChipIcon,
  DocumentTextIcon,
  ServerIcon,
  ShieldCheckIcon,
  WrenchScrewdriverIcon,
} from '@heroicons/react/24/outline';
import { useCallback, useEffect, useMemo, useState } from 'react';

import { formatRelativeTime, useDaemonHealth } from '../../../hooks/useDaemonHealth';
import {
  isTauri,
  openhumanAgentChat,
  openhumanDecryptSecret,
  openhumanDoctorModels,
  openhumanDoctorReport,
  openhumanEncryptSecret,
  openhumanGetConfig,
  openhumanGetDaemonHostConfig,
  openhumanGetIntegrationInfo,
  openhumanHardwareDiscover,
  openhumanHardwareIntrospect,
  openhumanListIntegrations,
  openhumanMigrateOpenclaw,
  openhumanModelsRefresh,
  openhumanServiceInstall,
  openhumanServiceStatus,
  openhumanServiceUninstall,
  openhumanSetDaemonHostConfig,
  openhumanUpdateMemorySettings,
  openhumanUpdateModelSettings,
  openhumanUpdateRuntimeSettings,
  openhumanUpdateTunnelSettings,
  runtimeDisableSkill,
  runtimeEnableSkill,
  runtimeIsSkillEnabled,
  runtimeListSkills,
  SkillSnapshot,
  TunnelConfig,
} from '../../../utils/tauriCommands';
import DaemonHealthIndicator from '../../daemon/DaemonHealthIndicator';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import ActionPanel, { PrimaryButton } from './components/ActionPanel';
import InputGroup, { CheckboxField, Field } from './components/InputGroup';
import SectionCard from './components/SectionCard';
import ValidatedField, { ValidatedSelect } from './components/ValidatedField';

const formatJson = (value: unknown) => JSON.stringify(value, null, 2);

const TauriCommandsPanel = () => {
  const { navigateBack } = useSettingsNavigation();
  const daemonHealth = useDaemonHealth();

  // View mode removed - always show all sections
  const [expandedSections] = useState<Set<string>>(
    new Set([
      'system-configuration',
      'runtime-execution',
      'security-data',
      'network-infrastructure',
      'development-operations',
      'interactive-tools',
    ])
  );

  // Output and error states
  const [output, setOutput] = useState<string>('');
  const [error, setError] = useState<string>('');

  // Form states (preserved from original)
  const [providerOverride, setProviderOverride] = useState<string>('');
  const [integrationName, setIntegrationName] = useState<string>('');
  const [hardwarePath, setHardwarePath] = useState<string>('');
  const [migrationSource, setMigrationSource] = useState<string>('');
  const [encryptInput, setEncryptInput] = useState<string>('');
  const [decryptInput, setDecryptInput] = useState<string>('');
  const [apiKey, setApiKey] = useState<string>('');
  const [apiUrl, setApiUrl] = useState<string>('');
  const [defaultProvider, setDefaultProvider] = useState<string>('');
  const [defaultModel, setDefaultModel] = useState<string>('');
  const [defaultTemp, setDefaultTemp] = useState<string>('0.7');
  const [tunnelProvider, setTunnelProvider] = useState<string>('none');
  const [cloudflareToken, setCloudflareToken] = useState<string>('');
  const [ngrokToken, setNgrokToken] = useState<string>('');
  const [tailscaleHostname, setTailscaleHostname] = useState<string>('');
  const [customCommand, setCustomCommand] = useState<string>('');
  const [memoryBackend, setMemoryBackend] = useState<string>('sqlite');
  const [memoryAutoSave, setMemoryAutoSave] = useState<boolean>(true);
  const [embeddingProvider, setEmbeddingProvider] = useState<string>('none');
  const [embeddingModel, setEmbeddingModel] = useState<string>('text-embedding-3-small');
  const [embeddingDims, setEmbeddingDims] = useState<string>('1536');
  const [runtimeKind, setRuntimeKind] = useState<string>('native');
  const [reasoningEnabled, setReasoningEnabled] = useState<boolean>(false);
  const [skills, setSkills] = useState<Array<{ snapshot: SkillSnapshot; enabled: boolean }>>([]);
  const [skillsLoading, setSkillsLoading] = useState<boolean>(false);
  const [chatInput, setChatInput] = useState<string>('');
  const [chatProvider, setChatProvider] = useState<string>('');
  const [chatModel, setChatModel] = useState<string>('');
  const [chatTemperature, setChatTemperature] = useState<string>('0.7');
  const [chatLog, setChatLog] = useState<Array<{ role: 'user' | 'agent'; text: string }>>([]);
  const [daemonShowTray, setDaemonShowTray] = useState<boolean>(true);
  const [daemonShowTrayLoaded, setDaemonShowTrayLoaded] = useState<boolean>(false);

  // Loading states
  const [operationLoading, setOperationLoading] = useState<string>('');

  // Enhanced System Configuration state management
  const [hasUnsavedChanges, setHasUnsavedChanges] = useState(false);
  const [originalConfig, setOriginalConfig] = useState<Record<string, unknown>>({});
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});
  const [lastSaveTime, setLastSaveTime] = useState<Date | null>(null);
  const [validationLoading, setValidationLoading] = useState(false);
  const [configLoaded, setConfigLoaded] = useState(false);

  const tauriAvailable = useMemo(() => isTauri(), []);
  const parseOptionalNumber = (value: string): number | null => {
    if (!value.trim()) {
      return null;
    }
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : null;
  };

  // Provider configurations for smart defaults
  const providerConfigs = useMemo(
    () => ({
      openai: {
        defaultUrl: 'https://api.openai.com/v1',
        keyPattern: /^sk-[a-zA-Z0-9]{32,}$/,
        models: ['gpt-4', 'gpt-4-turbo', 'gpt-4o', 'gpt-3.5-turbo'],
      },
      anthropic: {
        defaultUrl: 'https://api.anthropic.com',
        keyPattern: /^sk-ant-[a-zA-Z0-9_-]{32,}$/,
        models: ['claude-sonnet-4-5-20250929', 'claude-opus-4-6', 'claude-haiku-3-5'],
      },
      ollama: {
        defaultUrl: 'http://localhost:11434',
        keyPattern: null, // No API key required
        models: ['llama3', 'llama3:8b', 'codellama', 'mistral', 'phi3'],
      },
      groq: {
        defaultUrl: 'https://api.groq.com/openai/v1',
        keyPattern: /^gsk_[a-zA-Z0-9]{32,}$/,
        models: ['llama-3.1-70b-versatile', 'llama-3.1-8b-instant', 'mixtral-8x7b-32768'],
      },
      cohere: {
        defaultUrl: 'https://api.cohere.ai/v1',
        keyPattern: /^[a-zA-Z0-9]{32,}$/,
        models: ['command-r', 'command-r-plus', 'command-light'],
      },
    }),
    []
  );

  // Validation functions
  const validateApiKey = useCallback(
    (key: string, provider: string): string | null => {
      if (!provider || provider === 'none') return null;
      if (!key.trim() && provider && provider !== 'none' && provider !== 'ollama') {
        return 'API key is required for this provider';
      }
      if (
        key.trim() &&
        provider &&
        providerConfigs[provider as keyof typeof providerConfigs]?.keyPattern
      ) {
        const pattern = providerConfigs[provider as keyof typeof providerConfigs].keyPattern;
        if (pattern && !pattern.test(key)) {
          return `Invalid API key format for ${provider}`;
        }
      }
      return null;
    },
    [providerConfigs]
  );

  const validateApiUrl = useCallback((url: string): string | null => {
    if (!url.trim()) return null;
    try {
      const parsedUrl = new URL(url);
      if (!['http:', 'https:'].includes(parsedUrl.protocol)) {
        return 'URL must use HTTP or HTTPS protocol';
      }
      if (
        parsedUrl.protocol === 'http:' &&
        !parsedUrl.hostname.includes('localhost') &&
        !parsedUrl.hostname.includes('127.0.0.1')
      ) {
        return 'HTTP URLs are only allowed for localhost';
      }
      return null;
    } catch {
      return 'Invalid URL format';
    }
  }, []);

  const validateProvider = useCallback(
    (provider: string): string | null => {
      if (!provider.trim()) return null;
      const supportedProviders = Object.keys(providerConfigs);
      if (!supportedProviders.includes(provider)) {
        return `Unsupported provider. Supported: ${supportedProviders.join(', ')}`;
      }
      return null;
    },
    [providerConfigs]
  );

  const validateModel = useCallback(
    (model: string, provider: string): string | null => {
      if (!model.trim() || !provider.trim()) return null;
      const config = providerConfigs[provider as keyof typeof providerConfigs];
      if (config && !config.models.includes(model)) {
        return `Model not available for ${provider}. Try: ${config.models.slice(0, 3).join(', ')}`;
      }
      return null;
    },
    [providerConfigs]
  );

  const validateTemperature = useCallback((temp: string): string | null => {
    if (!temp.trim()) return null;
    const value = parseFloat(temp);
    if (isNaN(value)) return 'Temperature must be a number';
    if (value < 0 || value > 2) return 'Temperature must be between 0.0 and 2.0';
    return null;
  }, []);

  // Real-time validation
  const performValidation = useCallback(() => {
    const errors: Record<string, string> = {};

    const apiKeyError = validateApiKey(apiKey, defaultProvider);
    if (apiKeyError) errors.apiKey = apiKeyError;

    const apiUrlError = validateApiUrl(apiUrl);
    if (apiUrlError) errors.apiUrl = apiUrlError;

    const providerError = validateProvider(defaultProvider);
    if (providerError) errors.defaultProvider = providerError;

    const modelError = validateModel(defaultModel, defaultProvider);
    if (modelError) errors.defaultModel = modelError;

    const tempError = validateTemperature(defaultTemp);
    if (tempError) errors.defaultTemp = tempError;

    setFieldErrors(errors);
    return Object.keys(errors).length === 0;
  }, [
    apiKey,
    apiUrl,
    defaultProvider,
    defaultModel,
    defaultTemp,
    validateApiKey,
    validateApiUrl,
    validateProvider,
    validateModel,
    validateTemperature,
  ]);

  // Format timestamp for display
  const formatTime = useCallback((date: Date): string => {
    return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
  }, []);

  // Track changes
  useEffect(() => {
    if (!configLoaded) return;

    const currentConfig = {
      api_key: apiKey,
      api_url: apiUrl,
      default_provider: defaultProvider,
      default_model: defaultModel,
      default_temperature: defaultTemp,
    };

    const hasChanges = JSON.stringify(currentConfig) !== JSON.stringify(originalConfig);
    setHasUnsavedChanges(hasChanges);

    // Perform validation on changes
    performValidation();
  }, [
    apiKey,
    apiUrl,
    defaultProvider,
    defaultModel,
    defaultTemp,
    originalConfig,
    configLoaded,
    performValidation,
  ]);

  // Auto-populate URL based on provider
  useEffect(() => {
    if (
      defaultProvider &&
      !apiUrl &&
      providerConfigs[defaultProvider as keyof typeof providerConfigs]
    ) {
      const config = providerConfigs[defaultProvider as keyof typeof providerConfigs];
      setApiUrl(config.defaultUrl);
    }
  }, [defaultProvider, apiUrl, providerConfigs]);

  const run = async (fn: () => Promise<unknown>, operationName?: string) => {
    setError('');
    if (operationName) setOperationLoading(operationName);
    try {
      const result = await fn();
      setOutput(formatJson(result));
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
    } finally {
      setOperationLoading('');
    }
  };

  const runWithResult = async <T,>(
    fn: () => Promise<T>,
    operationName?: string
  ): Promise<T | null> => {
    setError('');
    if (operationName) setOperationLoading(operationName);
    try {
      const result = await fn();
      setOutput(formatJson(result));
      return result;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
      return null;
    } finally {
      setOperationLoading('');
    }
  };

  const loadConfig = async () => {
    const response = await runWithResult(() => openhumanGetConfig(), 'loadConfig');
    if (!response) {
      setError('Failed to load configuration');
      return;
    }
    try {
      const snapshot = response.result;
      const config = snapshot.config as Record<string, unknown>;

      // Extract model configuration
      const modelApiKey = (config.api_key as string) ?? '';
      const modelApiUrl = (config.api_url as string) ?? '';
      const modelProvider = (config.default_provider as string) ?? '';
      const modelModel = (config.default_model as string) ?? '';
      const modelTemp = String((config.default_temperature as number) ?? 0.7);

      // Set state
      setApiKey(modelApiKey);
      setApiUrl(modelApiUrl);
      setDefaultProvider(modelProvider);
      setDefaultModel(modelModel);
      setDefaultTemp(modelTemp);

      // Store original config for change tracking
      const systemConfig = {
        api_key: modelApiKey,
        api_url: modelApiUrl,
        default_provider: modelProvider,
        default_model: modelModel,
        default_temperature: modelTemp,
      };
      setOriginalConfig(systemConfig);
      setConfigLoaded(true);

      // Load other configuration sections
      const tunnel = (config.tunnel as Record<string, unknown>) ?? {};
      setTunnelProvider((tunnel.provider as string) ?? 'none');
      setCloudflareToken(((tunnel.cloudflare as Record<string, unknown>)?.token as string) ?? '');
      setNgrokToken(((tunnel.ngrok as Record<string, unknown>)?.auth_token as string) ?? '');
      setTailscaleHostname(
        ((tunnel.tailscale as Record<string, unknown>)?.hostname as string) ?? ''
      );
      setCustomCommand(((tunnel.custom as Record<string, unknown>)?.start_command as string) ?? '');

      const memory = (config.memory as Record<string, unknown>) ?? {};
      setMemoryBackend((memory.backend as string) ?? 'sqlite');
      setMemoryAutoSave((memory.auto_save as boolean) ?? true);
      setEmbeddingProvider((memory.embedding_provider as string) ?? 'none');
      setEmbeddingModel((memory.embedding_model as string) ?? 'text-embedding-3-small');
      setEmbeddingDims(String((memory.embedding_dimensions as number) ?? 1536));

      const runtime = (config.runtime as Record<string, unknown>) ?? {};
      setRuntimeKind((runtime.kind as string) ?? 'native');
      setReasoningEnabled((runtime.reasoning_enabled as boolean) ?? false);

      // Clear any previous errors
      setFieldErrors({});
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(`Failed to parse configuration: ${message}`);
    }
  };

  const loadDaemonHostConfig = useCallback(async () => {
    if (!tauriAvailable) {
      return;
    }

    try {
      const result = await openhumanGetDaemonHostConfig();
      setDaemonShowTray(result.result.show_tray);
      setDaemonShowTrayLoaded(true);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
    }
  }, [tauriAvailable]);

  const saveDaemonHostTraySetting = useCallback(
    async (showTray: boolean) => {
      if (!tauriAvailable) {
        return;
      }

      const previous = daemonShowTray;
      setDaemonShowTray(showTray);
      setOperationLoading('saveDaemonHostConfig');
      setError('');
      try {
        const result = await openhumanSetDaemonHostConfig(showTray);
        setOutput(formatJson(result));
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        setDaemonShowTray(previous);
        setError(message);
      } finally {
        setOperationLoading('');
      }
    },
    [daemonShowTray, tauriAvailable]
  );

  const buildTunnelConfig = (): TunnelConfig => {
    if (tunnelProvider === 'cloudflare') {
      return { provider: 'cloudflare', cloudflare: { token: cloudflareToken } };
    }
    if (tunnelProvider === 'ngrok') {
      return { provider: 'ngrok', ngrok: { auth_token: ngrokToken } };
    }
    if (tunnelProvider === 'tailscale') {
      return { provider: 'tailscale', tailscale: { hostname: tailscaleHostname || null } };
    }
    if (tunnelProvider === 'custom') {
      return { provider: 'custom', custom: { start_command: customCommand } };
    }
    return { provider: 'none' };
  };

  const saveModelSettings = async () => {
    // Pre-save validation
    if (!performValidation()) {
      setError('Please fix validation errors before saving');
      return;
    }

    setError('');
    setOperationLoading('saveModelSettings');

    try {
      const result = await openhumanUpdateModelSettings({
        api_key: apiKey.trim() ? apiKey : null,
        api_url: apiUrl.trim() ? apiUrl : null,
        default_provider: defaultProvider.trim() ? defaultProvider : null,
        default_model: defaultModel.trim() ? defaultModel : null,
        default_temperature: parseOptionalNumber(defaultTemp),
      });

      setOutput(formatJson(result));

      // Success feedback
      const now = new Date();
      setLastSaveTime(now);
      setHasUnsavedChanges(false);

      // Update original config
      setOriginalConfig({
        api_key: apiKey,
        api_url: apiUrl,
        default_provider: defaultProvider,
        default_model: defaultModel,
        default_temperature: defaultTemp,
      });
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      if (message.includes('API key')) {
        setFieldErrors(prev => ({ ...prev, apiKey: 'Invalid API key or authentication failed' }));
      } else if (message.includes('provider')) {
        setFieldErrors(prev => ({
          ...prev,
          defaultProvider: 'Provider not supported or misconfigured',
        }));
      } else if (message.includes('model')) {
        setFieldErrors(prev => ({
          ...prev,
          defaultModel: 'Model not available for this provider',
        }));
      }
      setError(message);
    } finally {
      setOperationLoading('');
    }
  };

  const testConnection = async () => {
    if (!performValidation()) {
      setError('Please fix validation errors before testing connection');
      return;
    }

    if (!defaultProvider || (!apiKey && defaultProvider !== 'ollama')) {
      setError('Provider and API key are required to test connection (except for Ollama)');
      return;
    }

    // Check if running in Tauri environment
    if (!isTauri()) {
      setError('Test Connection is only available in the desktop application');
      return;
    }

    setValidationLoading(true);
    setError('');

    try {
      // Add timeout to prevent infinite loading
      const timeoutPromise = new Promise((_, reject) =>
        setTimeout(() => reject(new Error('Connection test timed out after 30 seconds')), 30000)
      );

      // Test connection by attempting to refresh models with current settings
      const result = await Promise.race([
        openhumanModelsRefresh(defaultProvider, false),
        timeoutPromise,
      ]);

      setOutput(formatJson(result));

      // If we get here, connection is successful
      const successMessage = `Connection test successful for ${defaultProvider}`;
      setOutput(prev => prev + '\n\n' + successMessage);

      // Clear any previous connection errors
      setFieldErrors(prev => {
        const newErrors = { ...prev };
        delete newErrors.apiKey;
        delete newErrors.defaultProvider;
        return newErrors;
      });
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      console.error('Test connection error:', err);

      // Set provider-specific errors
      if (message.includes('authentication') || message.includes('401')) {
        setFieldErrors(prev => ({ ...prev, apiKey: 'Authentication failed - check API key' }));
      } else if (message.includes('provider') || message.includes('404')) {
        setFieldErrors(prev => ({ ...prev, defaultProvider: 'Provider not found or unavailable' }));
      } else if (message.includes('network') || message.includes('timeout')) {
        setFieldErrors(prev => ({
          ...prev,
          apiUrl: 'Network error - check API URL and connectivity',
        }));
      } else if (message.includes('Not running in Tauri')) {
        setFieldErrors(prev => ({
          ...prev,
          defaultProvider: 'Desktop application required for testing',
        }));
      }

      setError(`Connection test failed: ${message}`);
    } finally {
      setValidationLoading(false);
    }
  };

  const saveTunnelSettings = () =>
    run(() => openhumanUpdateTunnelSettings(buildTunnelConfig()), 'saveTunnelSettings');

  const saveMemorySettings = () =>
    run(
      () =>
        openhumanUpdateMemorySettings({
          backend: memoryBackend.trim() ? memoryBackend : null,
          auto_save: memoryAutoSave,
          embedding_provider: embeddingProvider.trim() ? embeddingProvider : null,
          embedding_model: embeddingModel.trim() ? embeddingModel : null,
          embedding_dimensions: parseOptionalNumber(embeddingDims),
        }),
      'saveMemorySettings'
    );

  const saveRuntimeSettings = () =>
    run(
      () =>
        openhumanUpdateRuntimeSettings({
          kind: runtimeKind.trim() ? runtimeKind : null,
          reasoning_enabled: reasoningEnabled,
        }),
      'saveRuntimeSettings'
    );

  const loadSkills = async () => {
    setSkillsLoading(true);
    try {
      const snapshots = await runtimeListSkills();
      const enriched = await Promise.all(
        snapshots.map(async snapshot => ({
          snapshot,
          enabled: await runtimeIsSkillEnabled(snapshot.skill_id),
        }))
      );
      setSkills(enriched);
      setOutput(formatJson(enriched));
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
    } finally {
      setSkillsLoading(false);
    }
  };

  const toggleSkill = async (skillId: string, nextEnabled: boolean) => {
    if (nextEnabled) {
      await run(() => runtimeEnableSkill(skillId), 'enableSkill');
    } else {
      await run(() => runtimeDisableSkill(skillId), 'disableSkill');
    }
    setSkills(prev =>
      prev.map(item =>
        item.snapshot.skill_id === skillId ? { ...item, enabled: nextEnabled } : item
      )
    );
  };

  const sendChat = async () => {
    if (!chatInput.trim()) {
      return;
    }
    const userMessage = chatInput.trim();
    setChatLog(prev => [...prev, { role: 'user', text: userMessage }]);
    setChatInput('');
    const response = await runWithResult(
      () =>
        openhumanAgentChat(
          userMessage,
          chatProvider.trim() ? chatProvider : undefined,
          chatModel.trim() ? chatModel : undefined,
          parseOptionalNumber(chatTemperature) ?? undefined
        ),
      'sendChat'
    );
    if (response) {
      setChatLog(prev => [...prev, { role: 'agent', text: response.result }]);
    }
  };

  // Always show all sections
  const isSectionVisible = () => {
    return true; // Always show all sections
  };

  useEffect(() => {
    void loadDaemonHostConfig();
  }, [loadDaemonHostConfig]);

  const isCollapsed = (sectionId: string) => {
    return !expandedSections.has(sectionId);
  };

  // Helper to check if a section is collapsed (currently unused but kept for future expansion)
  // const toggleSection = (sectionId: string) => {
  //   const newExpanded = new Set(expandedSections);
  //   if (newExpanded.has(sectionId)) {
  //     newExpanded.delete(sectionId);
  //   } else {
  //     newExpanded.add(sectionId);
  //   }
  //   setExpandedSections(newExpanded);
  // };

  return (
    <div className="h-full flex flex-col">
      <SettingsHeader title="Tauri Command Console" showBackButton={true} onBack={navigateBack} />

      <div className="flex-1 overflow-y-auto px-6 pb-12 space-y-10">
        {!tauriAvailable && (
          <div className="rounded-lg border border-amber-500/40 bg-amber-500/10 px-4 py-3 text-sm text-amber-200">
            Tauri runtime not detected. Commands will fail in browser mode.
          </div>
        )}

        {operationLoading && (
          <div className="flex items-center justify-end">
            <div className="flex items-center gap-2 text-sm text-gray-400">
              <div className="h-4 w-4 border-2 border-white/20 border-t-white rounded-full animate-spin" />
              {operationLoading}
            </div>
          </div>
        )}

        {/* Critical Path - Always visible in grid on desktop */}
        <div className="grid gap-8 lg:grid-cols-2">
          {/* Category 1: System Configuration */}
          {isSectionVisible() && (
            <SectionCard
              title="System Configuration"
              priority="infrastructure"
              icon={<CogIcon />}
              collapsible={true}
              defaultExpanded={!isCollapsed('system-configuration')}
              hasChanges={hasUnsavedChanges}
              loading={
                operationLoading === 'loadConfig' ||
                operationLoading === 'saveModelSettings' ||
                validationLoading
              }>
              <InputGroup
                title="Model API Keys"
                description="Configure your AI model provider settings with real-time validation">
                <ValidatedSelect
                  label="Default Provider"
                  value={defaultProvider}
                  onChange={value => {
                    setDefaultProvider(value);
                    // Auto-populate URL when provider changes
                    if (value && providerConfigs[value as keyof typeof providerConfigs]) {
                      const config = providerConfigs[value as keyof typeof providerConfigs];
                      if (
                        !apiUrl ||
                        Object.values(providerConfigs).some(c => c.defaultUrl === apiUrl)
                      ) {
                        setApiUrl(config.defaultUrl);
                      }
                    }
                  }}
                  options={[
                    {
                      value: '',
                      label: 'Select provider...',
                      description: 'Choose your AI service provider',
                    },
                    {
                      value: 'openai',
                      label: 'OpenAI',
                      description: 'GPT models with high performance',
                    },
                    {
                      value: 'anthropic',
                      label: 'Anthropic',
                      description: 'Claude models with safety focus',
                    },
                    {
                      value: 'ollama',
                      label: 'Ollama',
                      description: 'Local models, no API key needed',
                    },
                    {
                      value: 'groq',
                      label: 'Groq',
                      description: 'Fast inference with LPU acceleration',
                    },
                    {
                      value: 'cohere',
                      label: 'Cohere',
                      description: 'Enterprise-grade language models',
                    },
                  ]}
                  error={fieldErrors.defaultProvider}
                  required={true}
                  helpText="Primary AI provider for agent operations. Each provider offers different models with unique capabilities and pricing."
                />

                <ValidatedField
                  label="API Key"
                  value={apiKey}
                  onChange={setApiKey}
                  error={fieldErrors.apiKey}
                  required={!!defaultProvider && defaultProvider !== 'ollama'}
                  type="password"
                  placeholder={
                    defaultProvider === 'openai'
                      ? 'sk-...'
                      : defaultProvider === 'anthropic'
                        ? 'sk-ant-...'
                        : defaultProvider === 'groq'
                          ? 'gsk_...'
                          : defaultProvider === 'ollama'
                            ? 'Not required for Ollama'
                            : 'Enter your API key...'
                  }
                  helpText={
                    defaultProvider === 'ollama'
                      ? 'API key not required for Ollama (local models)'
                      : "Enter your AI provider's API key for authentication. Keep this secure and never share it publicly."
                  }
                  validation={
                    !apiKey
                      ? 'none'
                      : fieldErrors.apiKey
                        ? 'invalid'
                        : defaultProvider && validateApiKey(apiKey, defaultProvider) === null
                          ? 'valid'
                          : 'none'
                  }
                  disabled={defaultProvider === 'ollama'}
                />

                <ValidatedField
                  label="API Base URL"
                  value={apiUrl}
                  onChange={setApiUrl}
                  error={fieldErrors.apiUrl}
                  type="url"
                  placeholder="https://api.openai.com/v1"
                  helpText="Custom API endpoint for your AI provider. Auto-populated when you select a provider. Change only for custom deployments or proxy servers."
                  validation={
                    !apiUrl
                      ? 'none'
                      : fieldErrors.apiUrl
                        ? 'invalid'
                        : validateApiUrl(apiUrl) === null
                          ? 'valid'
                          : 'none'
                  }
                />

                <ValidatedSelect
                  label="Default Model"
                  value={defaultModel}
                  onChange={setDefaultModel}
                  options={[
                    {
                      value: '',
                      label: 'Select model...',
                      description: 'Choose specific model for this provider',
                    },
                    ...((defaultProvider &&
                      providerConfigs[defaultProvider as keyof typeof providerConfigs]?.models.map(
                        model => ({
                          value: model,
                          label: model,
                          description: model.includes('gpt-4')
                            ? 'Advanced reasoning, higher cost'
                            : model.includes('gpt-3.5')
                              ? 'Fast and economical'
                              : model.includes('claude')
                                ? 'Excellent analysis and safety'
                                : model.includes('llama')
                                  ? 'Open source, good performance'
                                  : model.includes('mixtral')
                                    ? 'Mixture of experts model'
                                    : 'High-quality language model',
                        })
                      )) ||
                      []),
                  ]}
                  error={fieldErrors.defaultModel}
                  helpText="Primary language model for agent interactions. Available models are filtered based on your selected provider."
                  disabled={!defaultProvider}
                  validation={
                    !defaultModel
                      ? 'none'
                      : fieldErrors.defaultModel
                        ? 'invalid'
                        : defaultProvider && validateModel(defaultModel, defaultProvider) === null
                          ? 'valid'
                          : 'none'
                  }
                />

                <ValidatedField
                  label="Temperature"
                  value={defaultTemp}
                  onChange={setDefaultTemp}
                  error={fieldErrors.defaultTemp}
                  type="number"
                  placeholder="0.7"
                  helpText="Controls randomness in AI responses (0.0-2.0). Lower values (0.1-0.3) for factual tasks, medium (0.5-0.8) for balanced responses, higher (0.8-1.5) for creative tasks."
                  validation={
                    !defaultTemp
                      ? 'none'
                      : fieldErrors.defaultTemp
                        ? 'invalid'
                        : validateTemperature(defaultTemp) === null
                          ? 'valid'
                          : 'none'
                  }
                />
              </InputGroup>

              <ActionPanel
                hasChanges={hasUnsavedChanges}
                success={lastSaveTime ? `Settings saved at ${formatTime(lastSaveTime)}` : false}
                error={Object.values(fieldErrors).find(Boolean)}>
                <PrimaryButton
                  onClick={loadConfig}
                  loading={operationLoading === 'loadConfig'}
                  variant="outline">
                  Load Config
                </PrimaryButton>
                <PrimaryButton
                  onClick={testConnection}
                  loading={validationLoading}
                  disabled={!defaultProvider || (!apiKey && defaultProvider !== 'ollama')}
                  variant="outline">
                  Test Connection
                </PrimaryButton>
                <PrimaryButton
                  onClick={saveModelSettings}
                  loading={operationLoading === 'saveModelSettings'}
                  disabled={Object.keys(fieldErrors).length > 0 || !hasUnsavedChanges}>
                  Save Settings
                </PrimaryButton>
              </ActionPanel>
            </SectionCard>
          )}

          {/* Category 2: Runtime & Execution */}
          {isSectionVisible() && (
            <SectionCard
              title="Runtime & Execution"
              priority="infrastructure"
              icon={<CpuChipIcon />}
              collapsible={true}
              defaultExpanded={!isCollapsed('runtime-execution')}
              hasChanges={false}
              loading={operationLoading === 'saveRuntimeSettings' || skillsLoading}>
              <InputGroup
                title="Runtime Settings"
                description="Configure V8 runtime and skill execution">
                <Field
                  label="Runtime Kind"
                  helpText="JavaScript execution environment for skills. 'native' uses V8 engine for maximum performance and compatibility. 'docker' provides isolation but requires Docker. 'wasm' offers security with some limitations.">
                  <input
                    className="w-full px-4 py-3 rounded-lg bg-stone-900/40 border border-stone-800/60 text-white placeholder-stone-400 focus:border-primary-500/50 focus:ring-2 focus:ring-primary-500/30 focus:outline-none transition-all duration-200"
                    placeholder="native"
                    value={runtimeKind}
                    onChange={event => setRuntimeKind(event.target.value)}
                  />
                </Field>
                <CheckboxField
                  label="Reasoning enabled"
                  helpText="Activates advanced step-by-step reasoning capabilities in AI responses. Improves accuracy for complex tasks but increases response time and token usage. Recommended for analytical and problem-solving tasks."
                  checked={reasoningEnabled}
                  onChange={setReasoningEnabled}
                />
              </InputGroup>

              <ActionPanel>
                <PrimaryButton
                  onClick={saveRuntimeSettings}
                  loading={operationLoading === 'saveRuntimeSettings'}>
                  Save Runtime Settings
                </PrimaryButton>
                <PrimaryButton onClick={loadSkills} loading={skillsLoading} variant="outline">
                  {skillsLoading ? 'Loading Skills…' : 'Load Skills'}
                </PrimaryButton>
              </ActionPanel>

              {skills.length > 0 && (
                <div className="space-y-4">
                  <h5 className="text-sm font-medium text-gray-300">Skills</h5>
                  <div className="grid gap-3 max-h-52 overflow-y-auto">
                    {skills.map(item => (
                      <div
                        key={item.snapshot.skill_id}
                        className="flex items-center justify-between rounded-lg border border-white/10 bg-white/5 backdrop-blur-sm px-4 py-3">
                        <div className="flex-1 min-w-0">
                          <div className="text-sm text-white font-medium">{item.snapshot.name}</div>
                          <div className="text-xs text-gray-400 truncate">
                            {item.snapshot.skill_id}
                          </div>
                        </div>
                        <CheckboxField
                          label={item.enabled ? 'Active' : 'Inactive'}
                          checked={item.enabled}
                          onChange={checked => toggleSkill(item.snapshot.skill_id, checked)}
                          className="text-xs ml-4 flex-shrink-0"
                        />
                      </div>
                    ))}
                  </div>
                </div>
              )}

              <InputGroup title="Agent Service Management">
                <div className="md:col-span-2">
                  <div className="space-y-4">
                    {/* Live Status Display */}
                    <div className="flex items-center justify-between p-3 rounded-lg bg-stone-900/40 border border-stone-800/60">
                      <div className="flex items-center gap-3">
                        <DaemonHealthIndicator size="md" />
                        <div>
                          <div className="text-white font-medium">
                            Agent Status: {daemonHealth.status}
                          </div>
                          <div className="text-xs text-gray-400">
                            Last update:{' '}
                            {daemonHealth.lastUpdate
                              ? formatRelativeTime(daemonHealth.lastUpdate)
                              : 'Never'}
                          </div>
                          {daemonHealth.healthSnapshot && (
                            <div className="text-xs text-gray-500">
                              PID: {daemonHealth.healthSnapshot.pid} • Uptime:{' '}
                              {daemonHealth.uptimeText}
                            </div>
                          )}
                        </div>
                      </div>
                      {daemonHealth.status === 'error' && (
                        <PrimaryButton
                          onClick={() => daemonHealth.restartDaemon()}
                          variant="outline"
                          loading={daemonHealth.isRecovering}>
                          Restart
                        </PrimaryButton>
                      )}
                    </div>

                    {/* Component Health */}
                    {daemonHealth.componentCount > 0 && (
                      <div className="grid grid-cols-2 gap-2 text-sm">
                        {Object.entries(daemonHealth.components).map(([name, health]) => (
                          <div
                            key={name}
                            className="flex items-center gap-2 p-2 rounded bg-stone-800/40">
                            <div
                              className={`w-2 h-2 rounded-full ${
                                health.status === 'ok'
                                  ? 'bg-green-500'
                                  : health.status === 'starting'
                                    ? 'bg-yellow-500'
                                    : 'bg-red-500'
                              }`}
                            />
                            <span className="capitalize text-gray-300">{name}</span>
                            {health.restart_count > 0 && (
                              <span className="text-xs text-yellow-400">
                                ({health.restart_count})
                              </span>
                            )}
                          </div>
                        ))}
                      </div>
                    )}

                    {/* Service Controls */}
                    <ActionPanel>
                      <PrimaryButton
                        onClick={() => daemonHealth.startDaemon()}
                        loading={operationLoading === 'serviceStart'}
                        disabled={daemonHealth.status === 'running'}>
                        Start
                      </PrimaryButton>
                      <PrimaryButton
                        onClick={() => daemonHealth.stopDaemon()}
                        loading={operationLoading === 'serviceStop'}
                        disabled={daemonHealth.status === 'disconnected'}
                        variant="outline">
                        Stop
                      </PrimaryButton>
                      <PrimaryButton
                        onClick={() => run(openhumanServiceStatus, 'serviceStatus')}
                        loading={operationLoading === 'serviceStatus'}
                        variant="outline">
                        Status
                      </PrimaryButton>
                      <PrimaryButton
                        onClick={() => run(openhumanServiceInstall, 'serviceInstall')}
                        loading={operationLoading === 'serviceInstall'}
                        variant="outline">
                        Install
                      </PrimaryButton>
                      <PrimaryButton
                        onClick={() => run(openhumanServiceUninstall, 'serviceUninstall')}
                        loading={operationLoading === 'serviceUninstall'}
                        variant="outline">
                        Uninstall
                      </PrimaryButton>
                    </ActionPanel>

                    {/* Auto-start Toggle */}
                    <div className="flex items-center justify-between p-3 rounded-lg bg-stone-800/40 border border-stone-700/60">
                      <div>
                        <div className="text-sm font-medium text-gray-300">Auto-start Agent</div>
                        <div className="text-xs text-gray-500">
                          Automatically start agent on app launch
                        </div>
                      </div>
                      <label className="relative inline-flex items-center cursor-pointer">
                        <input
                          type="checkbox"
                          className="sr-only peer"
                          checked={daemonHealth.isAutoStartEnabled}
                          onChange={e => daemonHealth.setAutoStart(e.target.checked)}
                        />
                        <div className="w-11 h-6 bg-gray-200 peer-focus:outline-none peer-focus:ring-4 peer-focus:ring-blue-300 dark:peer-focus:ring-blue-800 rounded-full peer dark:bg-gray-700 peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:border-gray-300 after:border after:rounded-full after:h-5 after:w-5 after:transition-all dark:border-gray-600 peer-checked:bg-blue-600"></div>
                      </label>
                    </div>

                    {/* Tray Toggle */}
                    <div className="flex items-center justify-between p-3 rounded-lg bg-stone-800/40 border border-stone-700/60">
                      <div>
                        <div className="text-sm font-medium text-gray-300">Show Daemon Tray</div>
                        <div className="text-xs text-gray-500">
                          Keep OpenHuman Core tray icon visible in daemon host mode
                        </div>
                        <div className="text-xs text-amber-400 mt-1">
                          Requires daemon host restart to fully apply
                        </div>
                      </div>
                      <label className="relative inline-flex items-center cursor-pointer">
                        <input
                          type="checkbox"
                          className="sr-only peer"
                          checked={daemonShowTray}
                          disabled={
                            !daemonShowTrayLoaded || operationLoading === 'saveDaemonHostConfig'
                          }
                          onChange={event => {
                            void saveDaemonHostTraySetting(event.target.checked);
                          }}
                        />
                        <div className="w-11 h-6 bg-gray-200 peer-focus:outline-none peer-focus:ring-4 peer-focus:ring-blue-300 dark:peer-focus:ring-blue-800 rounded-full peer dark:bg-gray-700 disabled:opacity-60 peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:border-gray-300 after:border after:rounded-full after:h-5 after:w-5 after:transition-all dark:border-gray-600 peer-checked:bg-blue-600"></div>
                      </label>
                    </div>

                    {/* Connection Info */}
                    {daemonHealth.connectionAttempts > 0 && (
                      <div className="p-3 rounded-lg bg-yellow-900/20 border border-yellow-500/30">
                        <div className="text-sm text-yellow-400">
                          Connection attempts: {daemonHealth.connectionAttempts}
                        </div>
                      </div>
                    )}
                  </div>
                </div>
              </InputGroup>
            </SectionCard>
          )}
        </div>

        {/* Category 3: Security & Data - Full width in basic mode, grid in advanced+ */}
        {isSectionVisible() && (
          <SectionCard
            title="Security & Data"
            priority="infrastructure"
            icon={<ShieldCheckIcon />}
            collapsible={true}
            defaultExpanded={!isCollapsed('security-data')}
            hasChanges={false}
            loading={
              operationLoading?.includes('Secret') ||
              operationLoading?.includes('Models') ||
              operationLoading?.includes('Integration')
            }>
            <div className="grid gap-6 lg:grid-cols-2">
              <InputGroup
                title="Secrets Management"
                description="Encrypt and decrypt sensitive data">
                <Field
                  label="Encrypt"
                  helpText="Convert sensitive data to encrypted format using the system's secure encryption. Useful for safely storing API keys, tokens, or other confidential information in configuration files."
                  fullWidth>
                  <textarea
                    className="w-full px-4 py-3 rounded-lg bg-stone-900/40 border border-stone-800/60 text-white placeholder-stone-400 focus:border-primary-500/50 focus:ring-2 focus:ring-primary-500/30 focus:outline-none transition-all duration-200 min-h-[90px] resize-y"
                    placeholder="Plaintext"
                    value={encryptInput}
                    onChange={event => setEncryptInput(event.target.value)}
                  />
                </Field>
                <Field
                  label="Decrypt"
                  helpText="Convert encrypted data back to readable format. Only works with data encrypted by this system. Use this to verify encrypted values or retrieve original content when needed."
                  fullWidth>
                  <textarea
                    className="w-full px-4 py-3 rounded-lg bg-stone-900/40 border border-stone-800/60 text-white placeholder-stone-400 focus:border-primary-500/50 focus:ring-2 focus:ring-primary-500/30 focus:outline-none transition-all duration-200 min-h-[90px] resize-y"
                    placeholder="Ciphertext"
                    value={decryptInput}
                    onChange={event => setDecryptInput(event.target.value)}
                  />
                </Field>
              </InputGroup>

              <div className="space-y-6">
                <InputGroup title="Models">
                  <Field
                    label="Provider Override"
                    helpText="Temporarily override the default AI provider for model operations. Leave empty to use the configured default provider. Useful for testing different providers or accessing provider-specific models."
                    fullWidth>
                    <input
                      className="w-full px-4 py-3 rounded-lg bg-stone-900/40 border border-stone-800/60 text-white placeholder-stone-400 focus:border-primary-500/50 focus:ring-2 focus:ring-primary-500/30 focus:outline-none transition-all duration-200"
                      placeholder="Provider override (optional)"
                      value={providerOverride}
                      onChange={event => setProviderOverride(event.target.value)}
                    />
                  </Field>
                </InputGroup>

                <InputGroup title="Integrations">
                  <Field
                    label="Integration Name"
                    helpText="Name of the platform integration to query or manage. Examples: 'telegram', 'gmail', 'discord'. Use this to get detailed information about specific platform connections and their status."
                    fullWidth>
                    <input
                      className="w-full px-4 py-3 rounded-lg bg-stone-900/40 border border-stone-800/60 text-white placeholder-stone-400 focus:border-primary-500/50 focus:ring-2 focus:ring-primary-500/30 focus:outline-none transition-all duration-200"
                      placeholder="Integration name"
                      value={integrationName}
                      onChange={event => setIntegrationName(event.target.value)}
                    />
                  </Field>
                </InputGroup>
              </div>
            </div>

            <ActionPanel>
              <PrimaryButton
                onClick={() => run(() => openhumanEncryptSecret(encryptInput), 'encryptSecret')}
                loading={operationLoading === 'encryptSecret'}
                disabled={!encryptInput.trim()}>
                Encrypt
              </PrimaryButton>
              <PrimaryButton
                onClick={() => run(() => openhumanDecryptSecret(decryptInput), 'decryptSecret')}
                loading={operationLoading === 'decryptSecret'}
                disabled={!decryptInput.trim()}
                variant="outline">
                Decrypt
              </PrimaryButton>
              <PrimaryButton
                onClick={() =>
                  run(
                    () => openhumanModelsRefresh(providerOverride || undefined, false),
                    'modelsRefresh'
                  )
                }
                loading={operationLoading === 'modelsRefresh'}>
                Refresh Models
              </PrimaryButton>
              <PrimaryButton
                onClick={() =>
                  run(
                    () => openhumanModelsRefresh(providerOverride || undefined, true),
                    'modelsForceRefresh'
                  )
                }
                loading={operationLoading === 'modelsForceRefresh'}
                variant="outline">
                Force Refresh
              </PrimaryButton>
              <PrimaryButton
                onClick={() => run(openhumanListIntegrations, 'listIntegrations')}
                loading={operationLoading === 'listIntegrations'}>
                List Integrations
              </PrimaryButton>
              <PrimaryButton
                onClick={() =>
                  run(() => openhumanGetIntegrationInfo(integrationName), 'getIntegrationInfo')
                }
                loading={operationLoading === 'getIntegrationInfo'}
                disabled={!integrationName.trim()}
                variant="outline">
                Get Integration Info
              </PrimaryButton>
            </ActionPanel>
          </SectionCard>
        )}

        {/* Category 4: Network & Infrastructure */}
        {isSectionVisible() && (
          <SectionCard
            title="Network & Infrastructure"
            priority="infrastructure"
            icon={<ServerIcon />}
            collapsible={true}
            defaultExpanded={!isCollapsed('network-infrastructure')}
            hasChanges={false}
            loading={operationLoading?.includes('Tunnel') || operationLoading?.includes('Memory')}>
            <div className="grid gap-8">
              <InputGroup
                title="Tunnel Configuration"
                description="Configure external tunnel providers">
                <Field label="Provider" fullWidth>
                  <select
                    className="w-full px-4 py-3 rounded-lg bg-stone-900/40 border border-stone-800/60 text-white focus:border-primary-500/50 focus:ring-2 focus:ring-primary-500/30 focus:outline-none transition-all duration-200"
                    value={tunnelProvider}
                    onChange={event => setTunnelProvider(event.target.value)}>
                    <option value="none">none</option>
                    <option value="cloudflare">cloudflare</option>
                    <option value="ngrok">ngrok</option>
                    <option value="tailscale">tailscale</option>
                    <option value="custom">custom</option>
                  </select>
                </Field>
                {tunnelProvider === 'cloudflare' && (
                  <Field label="Cloudflare Token" fullWidth>
                    <input
                      className="w-full px-4 py-3 rounded-lg bg-stone-900/40 border border-stone-800/60 text-white placeholder-stone-400 focus:border-primary-500/50 focus:ring-2 focus:ring-primary-500/30 focus:outline-none transition-all duration-200"
                      placeholder="cloudflare token"
                      value={cloudflareToken}
                      onChange={event => setCloudflareToken(event.target.value)}
                    />
                  </Field>
                )}
                {tunnelProvider === 'ngrok' && (
                  <Field label="Ngrok Token" fullWidth>
                    <input
                      className="w-full px-4 py-3 rounded-lg bg-stone-900/40 border border-stone-800/60 text-white placeholder-stone-400 focus:border-primary-500/50 focus:ring-2 focus:ring-primary-500/30 focus:outline-none transition-all duration-200"
                      placeholder="ngrok token"
                      value={ngrokToken}
                      onChange={event => setNgrokToken(event.target.value)}
                    />
                  </Field>
                )}
                {tunnelProvider === 'tailscale' && (
                  <Field label="Tailscale Hostname" fullWidth>
                    <input
                      className="w-full px-4 py-3 rounded-lg bg-stone-900/40 border border-stone-800/60 text-white placeholder-stone-400 focus:border-primary-500/50 focus:ring-2 focus:ring-primary-500/30 focus:outline-none transition-all duration-200"
                      placeholder="alpha.local"
                      value={tailscaleHostname}
                      onChange={event => setTailscaleHostname(event.target.value)}
                    />
                  </Field>
                )}
                {tunnelProvider === 'custom' && (
                  <Field label="Custom Start Command" fullWidth>
                    <input
                      className="w-full px-4 py-3 rounded-lg bg-stone-900/40 border border-stone-800/60 text-white placeholder-stone-400 focus:border-primary-500/50 focus:ring-2 focus:ring-primary-500/30 focus:outline-none transition-all duration-200"
                      placeholder="ngrok http 3000"
                      value={customCommand}
                      onChange={event => setCustomCommand(event.target.value)}
                    />
                  </Field>
                )}
              </InputGroup>
            </div>

            <InputGroup
              title="Memory Settings"
              description="Configure memory backend and embedding models">
              <Field
                label="Backend"
                helpText="Memory storage system for conversations and agent memory. 'sqlite' for local file storage (default), 'postgres' for scalable database, 'redis' for high-performance caching, 'neo4j' for graph relationships.">
                <input
                  className="w-full px-4 py-3 rounded-lg bg-stone-900/40 border border-stone-800/60 text-white placeholder-stone-400 focus:border-primary-500/50 focus:ring-2 focus:ring-primary-500/30 focus:outline-none transition-all duration-200"
                  placeholder="sqlite"
                  value={memoryBackend}
                  onChange={event => setMemoryBackend(event.target.value)}
                />
              </Field>
              <CheckboxField
                label="Auto-save"
                helpText="Automatically save conversation history and agent memory to the configured backend storage. Recommended for persistent memory across sessions and system restarts."
                checked={memoryAutoSave}
                onChange={setMemoryAutoSave}
              />
              <Field
                label="Embedding Provider"
                helpText="AI service for generating vector embeddings for semantic search and memory retrieval. 'openai' for high quality, 'cohere' for multilingual, 'huggingface' for local models, 'none' to disable.">
                <input
                  className="w-full px-4 py-3 rounded-lg bg-stone-900/40 border border-stone-800/60 text-white placeholder-stone-400 focus:border-primary-500/50 focus:ring-2 focus:ring-primary-500/30 focus:outline-none transition-all duration-200"
                  placeholder="openai"
                  value={embeddingProvider}
                  onChange={event => setEmbeddingProvider(event.target.value)}
                />
              </Field>
              <Field
                label="Embedding Model"
                helpText="Specific model for generating vector embeddings. OpenAI: 'text-embedding-3-small' (fast, cost-effective) or 'text-embedding-3-large' (higher accuracy). Must match your provider.">
                <input
                  className="w-full px-4 py-3 rounded-lg bg-stone-900/40 border border-stone-800/60 text-white placeholder-stone-400 focus:border-primary-500/50 focus:ring-2 focus:ring-primary-500/30 focus:outline-none transition-all duration-200"
                  placeholder="text-embedding-3-small"
                  value={embeddingModel}
                  onChange={event => setEmbeddingModel(event.target.value)}
                />
              </Field>
              <Field
                label="Embedding Dimensions"
                helpText="Vector size for embeddings. Must match your model: text-embedding-3-small supports 512-1536 (default 1536), text-embedding-3-large supports up to 3072. Higher dimensions = better accuracy, more storage.">
                <input
                  className="w-full px-4 py-3 rounded-lg bg-stone-900/40 border border-stone-800/60 text-white placeholder-stone-400 focus:border-primary-500/50 focus:ring-2 focus:ring-primary-500/30 focus:outline-none transition-all duration-200"
                  placeholder="1536"
                  value={embeddingDims}
                  onChange={event => setEmbeddingDims(event.target.value)}
                />
              </Field>
            </InputGroup>

            <ActionPanel>
              <PrimaryButton
                onClick={saveTunnelSettings}
                loading={operationLoading === 'saveTunnelSettings'}>
                Save Tunnel Settings
              </PrimaryButton>
              <PrimaryButton
                onClick={saveMemorySettings}
                loading={operationLoading === 'saveMemorySettings'}>
                Save Memory Settings
              </PrimaryButton>
            </ActionPanel>
          </SectionCard>
        )}

        {/* Category 5: Development & Operations */}
        {isSectionVisible() && (
          <SectionCard
            title="Development & Operations"
            priority="infrastructure"
            icon={<WrenchScrewdriverIcon />}
            collapsible={true}
            defaultExpanded={!isCollapsed('development-operations')}
            hasChanges={false}
            loading={
              operationLoading?.includes('Doctor') ||
              operationLoading?.includes('Hardware') ||
              operationLoading?.includes('Migration')
            }>
            <div className="grid gap-8 lg:grid-cols-2">
              <InputGroup title="Diagnostics" description="System health checks and model probing">
                <div className="md:col-span-2">
                  <ActionPanel>
                    <PrimaryButton
                      onClick={() => run(openhumanDoctorReport, 'doctorReport')}
                      loading={operationLoading === 'doctorReport'}>
                      Run Doctor Report
                    </PrimaryButton>
                    <PrimaryButton
                      onClick={() =>
                        run(
                          () => openhumanDoctorModels(providerOverride || undefined, true),
                          'probeModels'
                        )
                      }
                      loading={operationLoading === 'probeModels'}
                      variant="outline">
                      Probe Models
                    </PrimaryButton>
                  </ActionPanel>
                </div>
              </InputGroup>

              <InputGroup title="Hardware" description="Discover and introspect hardware devices">
                <Field
                  label="Device Path"
                  helpText="Full path to hardware device for introspection. Common paths: /dev/tty.usbmodem* (macOS USB), /dev/ttyUSB* (Linux), COM* (Windows). Use 'Discover Devices' to find available hardware."
                  fullWidth>
                  <input
                    className="w-full px-4 py-3 rounded-lg bg-stone-900/40 border border-stone-800/60 text-white placeholder-stone-400 focus:border-primary-500/50 focus:ring-2 focus:ring-primary-500/30 focus:outline-none transition-all duration-200"
                    placeholder="Device path (e.g. /dev/tty.usbmodem)"
                    value={hardwarePath}
                    onChange={event => setHardwarePath(event.target.value)}
                  />
                </Field>
              </InputGroup>
            </div>

            <InputGroup title="Migration" description="Migrate data from external sources">
              <Field
                label="Source Workspace"
                helpText="Path to existing agent workspace for data migration. Leave empty to migrate from default locations. Supports importing from OpenClaw, AutoGen, and other agent frameworks. Run dry-run first to preview changes."
                fullWidth>
                <input
                  className="w-full px-4 py-3 rounded-lg bg-stone-900/40 border border-stone-800/60 text-white placeholder-stone-400 focus:border-primary-500/50 focus:ring-2 focus:ring-primary-500/30 focus:outline-none transition-all duration-200"
                  placeholder="Source workspace (optional)"
                  value={migrationSource}
                  onChange={event => setMigrationSource(event.target.value)}
                />
              </Field>
            </InputGroup>

            <ActionPanel>
              <PrimaryButton
                onClick={() => run(openhumanHardwareDiscover, 'hardwareDiscover')}
                loading={operationLoading === 'hardwareDiscover'}>
                Discover Devices
              </PrimaryButton>
              <PrimaryButton
                onClick={() =>
                  run(() => openhumanHardwareIntrospect(hardwarePath), 'hardwareIntrospect')
                }
                loading={operationLoading === 'hardwareIntrospect'}
                disabled={!hardwarePath.trim()}
                variant="outline">
                Introspect Device
              </PrimaryButton>
              <PrimaryButton
                onClick={() =>
                  run(
                    () => openhumanMigrateOpenclaw(migrationSource || undefined, true),
                    'migrationDryRun'
                  )
                }
                loading={operationLoading === 'migrationDryRun'}>
                Dry Run Migration
              </PrimaryButton>
              <PrimaryButton
                onClick={() =>
                  run(
                    () => openhumanMigrateOpenclaw(migrationSource || undefined, false),
                    'runMigration'
                  )
                }
                loading={operationLoading === 'runMigration'}
                variant="outline">
                Run Migration
              </PrimaryButton>
            </ActionPanel>
          </SectionCard>
        )}

        {/* Category 6: Interactive Tools */}
        {isSectionVisible() && (
          <SectionCard
            title="Interactive Tools"
            priority="infrastructure"
            icon={<ChatBubbleLeftRightIcon />}
            collapsible={true}
            defaultExpanded={!isCollapsed('interactive-tools')}
            hasChanges={false}
            loading={operationLoading === 'sendChat'}>
            {/* Agent Chat - Preserve original styling */}
            <div className="space-y-6">
              <h4 className="text-lg font-medium text-white">Agent Chat</h4>
              <div className="grid gap-4 md:grid-cols-3">
                <label className="space-y-2 text-sm text-gray-300">
                  Provider Override
                  <input
                    className="w-full px-4 py-3 rounded-lg bg-stone-900/40 border border-stone-800/60 text-white placeholder-stone-400 focus:border-primary-500/50 focus:ring-2 focus:ring-primary-500/30 focus:outline-none transition-all duration-200"
                    placeholder="openai"
                    value={chatProvider}
                    onChange={event => setChatProvider(event.target.value)}
                  />
                  <p className="text-xs text-gray-400">
                    Override the default AI provider for this chat session. Leave empty to use
                    system default. Useful for testing different AI providers.
                  </p>
                </label>
                <label className="space-y-2 text-sm text-gray-300">
                  Model Override
                  <input
                    className="w-full px-4 py-3 rounded-lg bg-stone-900/40 border border-stone-800/60 text-white placeholder-stone-400 focus:border-primary-500/50 focus:ring-2 focus:ring-primary-500/30 focus:outline-none transition-all duration-200"
                    placeholder="gpt-4.1-mini"
                    value={chatModel}
                    onChange={event => setChatModel(event.target.value)}
                  />
                  <p className="text-xs text-gray-400">
                    Specific AI model for this chat session. Examples: gpt-4, gpt-3.5-turbo,
                    claude-3-sonnet. Leave empty for system default.
                  </p>
                </label>
                <label className="space-y-2 text-sm text-gray-300">
                  Temperature
                  <input
                    className="w-full px-4 py-3 rounded-lg bg-stone-900/40 border border-stone-800/60 text-white placeholder-stone-400 focus:border-primary-500/50 focus:ring-2 focus:ring-primary-500/30 focus:outline-none transition-all duration-200"
                    placeholder="0.7"
                    value={chatTemperature}
                    onChange={event => setChatTemperature(event.target.value)}
                  />
                  <p className="text-xs text-gray-400">
                    Creativity level for responses (0.0-2.0). Lower = more focused, higher = more
                    creative. Leave empty for system default.
                  </p>
                </label>
              </div>
              <div className="space-y-3">
                <textarea
                  className="w-full px-4 py-3 rounded-lg bg-stone-900/40 border border-stone-800/60 text-white placeholder-stone-400 focus:border-primary-500/50 focus:ring-2 focus:ring-primary-500/30 focus:outline-none transition-all duration-200 min-h-[120px] resize-y"
                  placeholder="Send a message to the agent..."
                  value={chatInput}
                  onChange={event => setChatInput(event.target.value)}
                />
                <p className="text-xs text-gray-400 leading-relaxed">
                  Direct chat interface with the AI agent. Test conversations, debug responses, or
                  interact with the agent using the configured settings above.
                </p>
              </div>
              <button
                className="bg-primary-600 hover:bg-primary-500 active:bg-primary-700 text-white font-medium px-6 py-3 rounded-lg transition-all duration-200 ease-in-out shadow-soft hover:shadow-medium focus:outline-none focus:ring-2 focus:ring-primary-500/50 focus:ring-offset-2 focus:ring-offset-black disabled:opacity-50 disabled:cursor-not-allowed"
                onClick={sendChat}>
                Send Message
              </button>
              {chatLog.length > 0 && (
                <div className="space-y-2 rounded-lg border border-white/10 bg-white/5 backdrop-blur-sm p-3">
                  {chatLog.map((entry, index) => (
                    <div
                      key={`${entry.role}-${index}`}
                      className={`text-sm ${
                        entry.role === 'user' ? 'text-white' : 'text-emerald-200'
                      }`}>
                      <span className="font-semibold uppercase text-[10px] tracking-wide">
                        {entry.role}
                      </span>
                      <div className="whitespace-pre-wrap">{entry.text}</div>
                    </div>
                  ))}
                </div>
              )}
            </div>

            {/* Output Console */}
            <div className="space-y-6">
              <div>
                <h4 className="text-lg font-medium text-white flex items-center gap-2">
                  <DocumentTextIcon className="h-5 w-5" />
                  Output Console
                </h4>
                <p className="text-sm text-gray-400 mt-2">
                  Real-time command results and system responses. Shows JSON output, error messages,
                  and operation status from all Tauri commands.
                </p>
              </div>
              {error && (
                <div className="rounded-lg border border-coral-500/40 bg-coral-500/10 px-4 py-3 text-sm text-coral-200">
                  {error}
                </div>
              )}
              <div className="space-y-3">
                <textarea
                  className="w-full px-4 py-3 rounded-lg bg-stone-900/40 border border-stone-800/60 text-white placeholder-stone-400 focus:border-primary-500/50 focus:ring-2 focus:ring-primary-500/30 focus:outline-none transition-all duration-200 min-h-[240px] font-mono text-xs resize-y"
                  value={output}
                  readOnly
                  placeholder="Command output will appear here..."
                />
                <p className="text-xs text-gray-400 leading-relaxed">
                  Read-only console showing formatted JSON responses, error details, and debugging
                  information from system operations.
                </p>
              </div>
            </div>
          </SectionCard>
        )}
      </div>
    </div>
  );
};

export default TauriCommandsPanel;
