/**
 * Unit tests for the tools loader system.
 * Tests loading, parsing, and caching functionality.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { clearToolsCache, loadTools, parseTools } from '../loader';
import type { ToolsConfig } from '../types';

// Mock the bundled tools markdown
vi.mock('../../../../../ai/TOOLS.md?raw', () => ({
  default: `# OpenHuman Tools

## Overview

OpenHuman has access to **4 tools** across **2 integrations**.

## Environment Configuration

### Development Environment
- **Access Level**: Full access to all tools
- **Rate Limits**: Relaxed for testing
- **Authentication**: Development credentials
- **Logging**: Verbose logging enabled

## Available Tools

### Telegram Tools

#### send_message

**Description**: Send a message to a Telegram chat or user

**Parameters**:
- **chat_id** (string) **(required)**: Telegram chat ID or username
- **message** (string) **(required)**: Message text to send
- **parse_mode** (string): Message formatting mode

#### get_chat_history

**Description**: Retrieve message history from a Telegram chat

**Parameters**:
- **chat_id** (string) **(required)**: Telegram chat ID or username
- **limit** (number): Number of messages to retrieve

### Gmail Tools

#### send_email

**Description**: Send an email via Gmail

**Parameters**:
- **to** (string) **(required)**: Recipient email address
- **subject** (string) **(required)**: Email subject line
- **body** (string) **(required)**: Email body content
`,
}));

// Mock localStorage
const localStorageMock = {
  getItem: vi.fn(),
  setItem: vi.fn(),
  removeItem: vi.fn(),
  clear: vi.fn(),
};

Object.defineProperty(window, 'localStorage', { value: localStorageMock });

// Mock fetch
global.fetch = vi.fn();

describe('Tools Loader', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    clearToolsCache();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe('parseTools', () => {
    it('should parse tools from markdown correctly', () => {
      const mockMarkdown = `# OpenHuman Tools

### Telegram Tools

#### send_message

**Description**: Send a message to a Telegram chat

**Parameters**:
- **chat_id** (string) **(required)**: Chat ID
- **message** (string) **(required)**: Message text

#### get_history

**Description**: Get chat history

**Parameters**:
- *None*
`;

      const config = parseTools(mockMarkdown, false);

      expect(config.tools).toHaveLength(2);
      expect(config.tools[0]).toMatchObject({
        name: 'send_message',
        description: 'Send a message to a Telegram chat',
        skillId: 'telegram',
      });

      expect(config.tools[0].inputSchema.properties).toHaveProperty('chat_id');
      expect(config.tools[0].inputSchema.properties).toHaveProperty('message');
      expect(config.tools[0].inputSchema.required).toContain('chat_id');
      expect(config.tools[0].inputSchema.required).toContain('message');

      expect(config.tools[1]).toMatchObject({
        name: 'get_history',
        description: 'Get chat history',
        skillId: 'telegram',
      });

      expect(config.tools[1].inputSchema.properties).toEqual({});
    });

    it('should group tools by skill correctly', () => {
      const mockMarkdown = `# OpenHuman Tools

### Telegram Tools

#### send_message
**Description**: Send message
**Parameters**:
- *None*

### Gmail Tools

#### send_email
**Description**: Send email
**Parameters**:
- *None*
`;

      const config = parseTools(mockMarkdown, false);

      expect(config.skillGroups).toHaveProperty('telegram');
      expect(config.skillGroups).toHaveProperty('gmail');
      expect(config.skillGroups.telegram.tools).toHaveLength(1);
      expect(config.skillGroups.gmail.tools).toHaveLength(1);
    });

    it('should generate statistics correctly', () => {
      const mockMarkdown = `# OpenHuman Tools

### Telegram Tools

#### send_message
**Description**: Send message
**Parameters**:
- *None*

#### get_history
**Description**: Get history
**Parameters**:
- *None*

### Gmail Tools

#### send_email
**Description**: Send email
**Parameters**:
- *None*
`;

      const config = parseTools(mockMarkdown, false);

      expect(config.statistics.totalTools).toBe(3);
      expect(config.statistics.activeSkills).toBe(2);
      expect(config.statistics.toolsByCategory.communication).toBe(3);
    });

    it('should handle empty markdown gracefully', () => {
      const config = parseTools('', true);

      expect(config.tools).toHaveLength(0);
      expect(config.skillGroups).toEqual({});
      expect(config.statistics.totalTools).toBe(0);
      expect(config.isDefault).toBe(true);
    });
  });

  describe('loadTools', () => {
    it('should load from cache when available', async () => {
      const mockConfig: ToolsConfig = {
        raw: 'cached markdown',
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

      const cacheEntry = { config: mockConfig, timestamp: Date.now(), version: '1.0.0' };

      localStorageMock.getItem.mockReturnValue(JSON.stringify(cacheEntry));

      const result = await loadTools();

      expect(result).toEqual(mockConfig);
      expect(fetch).not.toHaveBeenCalled();
    });

    it('should load from GitHub when cache is expired', async () => {
      const expiredCacheEntry = {
        config: {} as ToolsConfig,
        timestamp: Date.now() - 1000 * 60 * 60, // 1 hour ago
        version: '1.0.0',
      };

      localStorageMock.getItem.mockReturnValue(JSON.stringify(expiredCacheEntry));

      (fetch as any).mockResolvedValue({ ok: true, text: () => Promise.resolve('# Remote Tools') });

      const result = await loadTools();

      expect(fetch).toHaveBeenCalledWith(
        'https://raw.githubusercontent.com/openhumanxyz/openhuman/refs/heads/main/ai/TOOLS.md'
      );
      expect(result.raw).toBe('# Remote Tools');
      expect(result.isDefault).toBe(false);
    });

    it('should fallback to bundled tools when GitHub fails', async () => {
      localStorageMock.getItem.mockReturnValue(null);

      (fetch as any).mockRejectedValue(new Error('Network error'));

      const result = await loadTools();

      expect(result.isDefault).toBe(true);
      expect(result.tools).toHaveLength(4); // From mocked bundled tools
    });

    it('should cache the loaded configuration', async () => {
      localStorageMock.getItem.mockReturnValue(null);

      (fetch as any).mockResolvedValue({ ok: true, text: () => Promise.resolve('# Remote Tools') });

      await loadTools();

      expect(localStorageMock.setItem).toHaveBeenCalledWith(
        'openhuman.tools.cache',
        expect.stringContaining('"version":"1.0.0"')
      );
    });
  });

  describe('clearToolsCache', () => {
    it('should clear localStorage cache', () => {
      clearToolsCache();

      expect(localStorageMock.removeItem).toHaveBeenCalledWith('openhuman.tools.cache');
    });

    it('should handle localStorage errors gracefully', () => {
      localStorageMock.removeItem.mockImplementation(() => {
        throw new Error('Storage error');
      });

      expect(() => clearToolsCache()).not.toThrow();
    });
  });
});
