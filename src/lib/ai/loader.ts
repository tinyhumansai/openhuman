/**
 * Unified AI Configuration Loader
 *
 * Provides a single interface for loading both SOUL and TOOLS configurations
 * with parallel loading, caching, and fallback strategies.
 */
import { clearSoulCache, loadSoul } from './soul/loader';
import type { SoulConfig } from './soul/types';
import { clearToolsCache, loadTools } from './tools/loader';
import type { ToolsConfig } from './tools/types';
import type {
  AIConfig,
  AIConfigCacheEntry,
  AIConfigLoadOptions,
  AIConfigLoadResult,
  AIConfigMetadata,
} from './types';

const AI_CACHE_KEY = 'openhuman.ai.cache';
const AI_CACHE_TTL = 1000 * 60 * 30; // 30 minutes
const CACHE_VERSION = '1.0.0';

let cachedAIConfig: AIConfig | null = null;

/**
 * Load complete AI configuration (SOUL + TOOLS) with caching and parallel loading
 */
export async function loadAIConfig(options: AIConfigLoadOptions = {}): Promise<AIConfig> {
  const { forceRefresh = false, includeMetadata = true, timeout = 30000 } = options;

  const startTime = Date.now();

  // Check memory cache first (unless force refresh)
  if (!forceRefresh && cachedAIConfig) {
    return cachedAIConfig;
  }

  // Check localStorage cache (unless force refresh)
  if (!forceRefresh) {
    try {
      const cached = localStorage.getItem(AI_CACHE_KEY);
      if (cached) {
        const parsed = JSON.parse(cached) as AIConfigCacheEntry;
        if (Date.now() - parsed.timestamp < AI_CACHE_TTL && parsed.version === CACHE_VERSION) {
          cachedAIConfig = parsed.config;
          return parsed.config;
        }
      }
    } catch {
      // Ignore cache errors
    }
  }

  // Force clear caches if refresh requested
  if (forceRefresh) {
    clearSoulCache();
    clearToolsCache();
  }

  // Load both configurations in parallel
  const [soulResult, toolsResult] = await Promise.allSettled([
    loadWithTimeout(loadSoul(), timeout),
    loadWithTimeout(loadTools(), timeout),
  ]);

  // Extract results and handle errors
  let soul: SoulConfig;
  let tools: ToolsConfig;
  const errors: string[] = [];

  if (soulResult.status === 'fulfilled') {
    soul = soulResult.value;
  } else {
    errors.push(`Soul loading failed: ${soulResult.reason}`);
    // Create fallback soul config
    soul = createFallbackSoulConfig();
  }

  if (toolsResult.status === 'fulfilled') {
    tools = toolsResult.value;
  } else {
    errors.push(`Tools loading failed: ${toolsResult.reason}`);
    // Create fallback tools config
    tools = createFallbackToolsConfig();
  }

  // Generate metadata
  const endTime = Date.now();
  const metadata: AIConfigMetadata = {
    loadedAt: endTime,
    loadingDuration: endTime - startTime,
    hasFallbacks: soul.isDefault || tools.isDefault,
    sources: { soul: getSoulSource(soul), tools: getToolsSource(tools) },
  };

  if (errors.length > 0 && includeMetadata) {
    metadata.errors = errors;
  }

  // Combine into unified config
  const config: AIConfig = { soul, tools, metadata };

  // Cache the result
  cachedAIConfig = config;
  try {
    const cacheEntry: AIConfigCacheEntry = {
      config,
      timestamp: Date.now(),
      version: CACHE_VERSION,
    };
    localStorage.setItem(AI_CACHE_KEY, JSON.stringify(cacheEntry));
  } catch {
    // Ignore storage errors
  }

  return config;
}

/**
 * Load AI configuration with detailed result information
 */
export async function loadAIConfigWithResult(
  options: AIConfigLoadOptions = {}
): Promise<AIConfigLoadResult> {
  const startTime = Date.now();

  try {
    const config = await loadAIConfig(options);
    const endTime = Date.now();

    return {
      config,
      success: true,
      errors: config.metadata.errors || [],
      duration: endTime - startTime,
    };
  } catch (error) {
    const endTime = Date.now();
    const errorMessage = error instanceof Error ? error.message : 'Unknown error';

    return {
      config: createFallbackAIConfig(),
      success: false,
      errors: [errorMessage],
      duration: endTime - startTime,
    };
  }
}

/**
 * Refresh only the SOUL configuration
 */
export async function refreshSoul(): Promise<SoulConfig> {
  clearSoulCache();
  const soul = await loadSoul();

  // Update cached AI config if it exists
  if (cachedAIConfig) {
    cachedAIConfig.soul = soul;
    cachedAIConfig.metadata.loadedAt = Date.now();
    cachedAIConfig.metadata.hasFallbacks = soul.isDefault || cachedAIConfig.tools.isDefault;
  }

  // Update localStorage cache
  try {
    if (cachedAIConfig) {
      const cacheEntry: AIConfigCacheEntry = {
        config: cachedAIConfig,
        timestamp: Date.now(),
        version: CACHE_VERSION,
      };
      localStorage.setItem(AI_CACHE_KEY, JSON.stringify(cacheEntry));
    }
  } catch {
    // Ignore storage errors
  }

  return soul;
}

/**
 * Refresh only the TOOLS configuration
 */
export async function refreshTools(): Promise<ToolsConfig> {
  clearToolsCache();
  const tools = await loadTools();

  // Update cached AI config if it exists
  if (cachedAIConfig) {
    cachedAIConfig.tools = tools;
    cachedAIConfig.metadata.loadedAt = Date.now();
    cachedAIConfig.metadata.hasFallbacks = cachedAIConfig.soul.isDefault || tools.isDefault;
  }

  // Update localStorage cache
  try {
    if (cachedAIConfig) {
      const cacheEntry: AIConfigCacheEntry = {
        config: cachedAIConfig,
        timestamp: Date.now(),
        version: CACHE_VERSION,
      };
      localStorage.setItem(AI_CACHE_KEY, JSON.stringify(cacheEntry));
    }
  } catch {
    // Ignore storage errors
  }

  return tools;
}

/**
 * Refresh all AI configuration
 */
export async function refreshAll(): Promise<AIConfig> {
  return loadAIConfig({ forceRefresh: true });
}

/**
 * Clear all AI configuration caches
 */
export function clearAICache(): void {
  cachedAIConfig = null;
  clearSoulCache();
  clearToolsCache();
  try {
    localStorage.removeItem(AI_CACHE_KEY);
  } catch {
    // Ignore storage errors
  }
}

/**
 * Get current AI configuration from cache (if available)
 */
export function getCachedAIConfig(): AIConfig | null {
  return cachedAIConfig;
}

/**
 * Check if AI configuration is cached
 */
export function isAIConfigCached(): boolean {
  return cachedAIConfig !== null;
}

/**
 * Utility functions
 */
async function loadWithTimeout<T>(promise: Promise<T>, timeoutMs: number): Promise<T> {
  return Promise.race([
    promise,
    new Promise<never>((_, reject) =>
      setTimeout(() => reject(new Error(`Operation timed out after ${timeoutMs}ms`)), timeoutMs)
    ),
  ]);
}

function getSoulSource(soul: SoulConfig): AIConfigMetadata['sources']['soul'] {
  if (soul.isDefault) return 'bundled';
  // We can't easily determine the exact source, so we default to github for remote
  return 'github';
}

function getToolsSource(tools: ToolsConfig): AIConfigMetadata['sources']['tools'] {
  if (tools.isDefault) return 'bundled';
  // We can't easily determine the exact source, so we default to github for remote
  return 'github';
}

function createFallbackSoulConfig(): SoulConfig {
  return {
    raw: '# Fallback SOUL Configuration\n\nThis is a fallback configuration used when the main SOUL config cannot be loaded.',
    identity: {
      name: 'OpenHuman Assistant',
      description: 'AI assistant with fallback configuration',
    },
    personality: [],
    voiceTone: [],
    behaviors: [],
    safetyRules: [],
    interactions: [],
    memorySettings: { remember: [] },
    emergencyResponses: [],
    isDefault: true,
    loadedAt: Date.now(),
  };
}

function createFallbackToolsConfig(): ToolsConfig {
  return {
    raw: '# Fallback TOOLS Configuration\n\nThis is a fallback configuration used when the main TOOLS config cannot be loaded.',
    tools: [],
    skillGroups: {},
    categories: {},
    environments: {},
    statistics: {
      totalTools: 0,
      activeSkills: 0,
      categoriesCount: 0,
      toolsByCategory: {},
      skillsByCategory: {},
    },
    isDefault: true,
    loadedAt: Date.now(),
  };
}

function createFallbackAIConfig(): AIConfig {
  const soul = createFallbackSoulConfig();
  const tools = createFallbackToolsConfig();

  return {
    soul,
    tools,
    metadata: {
      loadedAt: Date.now(),
      loadingDuration: 0,
      hasFallbacks: true,
      sources: { soul: 'bundled', tools: 'bundled' },
      errors: ['Failed to load AI configuration, using fallbacks'],
    },
  };
}
