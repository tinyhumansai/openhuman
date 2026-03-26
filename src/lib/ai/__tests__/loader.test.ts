/**
 * Unit tests for the unified AI loader system.
 * Tests loading, parallel execution, and error handling.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { clearAICache, loadAIConfig, refreshAll, refreshSoul, refreshTools } from '../loader';
// Import the mocked functions
import { clearSoulCache, loadSoul } from '../soul/loader';
import type { SoulConfig } from '../soul/types';
import { clearToolsCache, loadTools } from '../tools/loader';
import type { ToolsConfig } from '../tools/types';

// Mock the individual loaders
vi.mock('../soul/loader', () => ({ loadSoul: vi.fn(), clearSoulCache: vi.fn() }));

vi.mock('../tools/loader', () => ({ loadTools: vi.fn(), clearToolsCache: vi.fn() }));

// Mock localStorage
const localStorageMock = {
  getItem: vi.fn(),
  setItem: vi.fn(),
  removeItem: vi.fn(),
  clear: vi.fn(),
};

Object.defineProperty(window, 'localStorage', { value: localStorageMock });

describe('Unified AI Loader', () => {
  const mockSoulConfig: SoulConfig = {
    raw: 'soul markdown',
    identity: { name: 'Test', description: 'Test soul' },
    personality: [],
    voiceTone: [],
    behaviors: [],
    safetyRules: [],
    interactions: [],
    memorySettings: { remember: [] },
    emergencyResponses: [],
    isDefault: false,
    loadedAt: Date.now(),
  };

  const mockToolsConfig: ToolsConfig = {
    raw: 'tools markdown',
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
    isDefault: false,
    loadedAt: Date.now(),
  };

  beforeEach(() => {
    vi.clearAllMocks();
    clearAICache();
    (loadSoul as any).mockResolvedValue(mockSoulConfig);
    (loadTools as any).mockResolvedValue(mockToolsConfig);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe('loadAIConfig', () => {
    it('should load both SOUL and TOOLS configurations', async () => {
      const config = await loadAIConfig();

      expect(config.soul).toEqual(mockSoulConfig);
      expect(config.tools).toEqual(mockToolsConfig);
      expect(config.metadata.hasFallbacks).toBe(false);
      expect(config.metadata.loadingDuration).toBeGreaterThan(0);
    });

    it('should return cached config on subsequent calls', async () => {
      // First call
      await loadAIConfig();

      // Second call should use cache
      const config = await loadAIConfig();

      expect(loadSoul).toHaveBeenCalledTimes(1);
      expect(loadTools).toHaveBeenCalledTimes(1);
      expect(config.soul).toEqual(mockSoulConfig);
      expect(config.tools).toEqual(mockToolsConfig);
    });

    it('should use localStorage cache when available', async () => {
      const cachedConfig = {
        soul: mockSoulConfig,
        tools: mockToolsConfig,
        metadata: {
          loadedAt: Date.now(),
          loadingDuration: 100,
          hasFallbacks: false,
          sources: { soul: 'github', tools: 'github' },
        },
      };

      const cacheEntry = { config: cachedConfig, timestamp: Date.now(), version: '1.0.0' };

      localStorageMock.getItem.mockReturnValue(JSON.stringify(cacheEntry));

      const config = await loadAIConfig();

      expect(config).toEqual(cachedConfig);
      expect(loadSoul).not.toHaveBeenCalled();
      expect(loadTools).not.toHaveBeenCalled();
    });

    it('should handle SOUL loading failure gracefully', async () => {
      (loadSoul as any).mockRejectedValue(new Error('Soul loading failed'));

      const config = await loadAIConfig();

      expect(config.soul.isDefault).toBe(true);
      expect(config.tools).toEqual(mockToolsConfig);
      expect(config.metadata.hasFallbacks).toBe(true);
      expect(config.metadata.errors).toContain('Soul loading failed: Error: Soul loading failed');
    });

    it('should handle TOOLS loading failure gracefully', async () => {
      (loadTools as any).mockRejectedValue(new Error('Tools loading failed'));

      const config = await loadAIConfig();

      expect(config.soul).toEqual(mockSoulConfig);
      expect(config.tools.isDefault).toBe(true);
      expect(config.metadata.hasFallbacks).toBe(true);
      expect(config.metadata.errors).toContain('Tools loading failed: Error: Tools loading failed');
    });

    it('should handle both loading failures gracefully', async () => {
      (loadSoul as any).mockRejectedValue(new Error('Soul error'));
      (loadTools as any).mockRejectedValue(new Error('Tools error'));

      const config = await loadAIConfig();

      expect(config.soul.isDefault).toBe(true);
      expect(config.tools.isDefault).toBe(true);
      expect(config.metadata.hasFallbacks).toBe(true);
      expect(config.metadata.errors).toHaveLength(2);
    });

    it('should force refresh when requested', async () => {
      // First load to populate cache
      await loadAIConfig();

      // Reset mocks
      vi.clearAllMocks();
      (loadSoul as any).mockResolvedValue(mockSoulConfig);
      (loadTools as any).mockResolvedValue(mockToolsConfig);

      // Force refresh
      await loadAIConfig({ forceRefresh: true });

      expect(clearSoulCache).toHaveBeenCalled();
      expect(clearToolsCache).toHaveBeenCalled();
      expect(loadSoul).toHaveBeenCalled();
      expect(loadTools).toHaveBeenCalled();
    });

    it('should handle timeout correctly', async () => {
      // Make loadSoul take a long time
      (loadSoul as any).mockImplementation(
        () => new Promise(resolve => setTimeout(() => resolve(mockSoulConfig), 100))
      );

      const config = await loadAIConfig({ timeout: 50 });

      expect(config.soul.isDefault).toBe(true); // Should use fallback due to timeout
      expect(config.metadata.errors).toContain(
        'Soul loading failed: Error: Operation timed out after 50ms'
      );
    }, 10000);
  });

  describe('refreshSoul', () => {
    it('should refresh only the SOUL configuration', async () => {
      // Initial load
      await loadAIConfig();

      // Reset mocks
      vi.clearAllMocks();
      const newSoulConfig = {
        ...mockSoulConfig,
        identity: { name: 'Updated', description: 'Updated' },
      };
      (loadSoul as any).mockResolvedValue(newSoulConfig);

      const result = await refreshSoul();

      expect(clearSoulCache).toHaveBeenCalled();
      expect(loadSoul).toHaveBeenCalled();
      expect(clearToolsCache).not.toHaveBeenCalled();
      expect(loadTools).not.toHaveBeenCalled();
      expect(result).toEqual(newSoulConfig);
    });
  });

  describe('refreshTools', () => {
    it('should refresh only the TOOLS configuration', async () => {
      // Initial load
      await loadAIConfig();

      // Reset mocks
      vi.clearAllMocks();
      const newToolsConfig = {
        ...mockToolsConfig,
        statistics: { ...mockToolsConfig.statistics, totalTools: 10 },
      };
      (loadTools as any).mockResolvedValue(newToolsConfig);

      const result = await refreshTools();

      expect(clearToolsCache).toHaveBeenCalled();
      expect(loadTools).toHaveBeenCalled();
      expect(clearSoulCache).not.toHaveBeenCalled();
      expect(loadSoul).not.toHaveBeenCalled();
      expect(result).toEqual(newToolsConfig);
    });
  });

  describe('refreshAll', () => {
    it('should refresh both configurations', async () => {
      const config = await refreshAll();

      expect(clearSoulCache).toHaveBeenCalled();
      expect(clearToolsCache).toHaveBeenCalled();
      expect(loadSoul).toHaveBeenCalled();
      expect(loadTools).toHaveBeenCalled();
      expect(config.soul).toEqual(mockSoulConfig);
      expect(config.tools).toEqual(mockToolsConfig);
    });
  });

  describe('clearAICache', () => {
    it('should clear all caches', () => {
      clearAICache();

      expect(clearSoulCache).toHaveBeenCalled();
      expect(clearToolsCache).toHaveBeenCalled();
      expect(localStorageMock.removeItem).toHaveBeenCalledWith('openhuman.ai.cache');
    });
  });
});
