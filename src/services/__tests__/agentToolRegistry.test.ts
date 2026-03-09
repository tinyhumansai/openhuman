import { describe, test, expect, beforeEach, vi, type Mock } from 'vitest';
import { AgentToolRegistry } from '../agentToolRegistry';
import { invoke } from '@tauri-apps/api/core';

// Mock Tauri invoke
vi.mock('@tauri-apps/api/core');

describe('AgentToolRegistry', () => {
  let service: AgentToolRegistry;
  const mockInvoke = invoke as Mock;

  beforeEach(() => {
    service = AgentToolRegistry.getInstance();
    vi.clearAllMocks();
    service.clearCache(); // Clear cache between tests
  });

  describe('loadToolSchemas', () => {
    test('should load tool schemas from Tauri using ZeroClaw format', async () => {
      const mockSchemas = [
        {
          type: "function",
          function: {
            name: "github_list_issues",
            description: "List GitHub issues for a repository",
            parameters: {
              type: "object",
              properties: {
                owner: { type: "string", description: "Repository owner" },
                repo: { type: "string", description: "Repository name" }
              },
              required: ["owner", "repo"]
            }
          }
        },
        {
          type: "function",
          function: {
            name: "notion_create_page",
            description: "Create a new Notion page",
            parameters: {
              type: "object",
              properties: {
                title: { type: "string", description: "Page title" },
                content: { type: "string", description: "Page content" }
              },
              required: ["title"]
            }
          }
        }
      ];

      mockInvoke.mockResolvedValue(mockSchemas);

      const schemas = await service.loadToolSchemas();

      expect(schemas).toHaveLength(2);
      expect(schemas[0].function.name).toBe("github_list_issues");
      expect(schemas[1].function.name).toBe("notion_create_page");
      expect(mockInvoke).toHaveBeenCalledWith('runtime_get_tool_schemas');
    });

    test('should cache tool schemas to avoid repeated calls', async () => {
      const mockSchemas = [
        {
          type: "function",
          function: {
            name: "test_tool",
            description: "Test tool",
            parameters: { type: "object", properties: {} }
          }
        }
      ];

      mockInvoke.mockResolvedValue(mockSchemas);

      // First call
      const schemas1 = await service.loadToolSchemas();
      // Second call
      const schemas2 = await service.loadToolSchemas();

      expect(schemas1).toEqual(schemas2);
      // Should only invoke Tauri once due to caching (TTL = 5 minutes)
      expect(mockInvoke).toHaveBeenCalledTimes(1);
    });

    test('should force reload when requested', async () => {
      const mockSchemas = [
        {
          type: "function",
          function: {
            name: "test_tool",
            description: "Test tool",
            parameters: { type: "object", properties: {} }
          }
        }
      ];

      mockInvoke.mockResolvedValue(mockSchemas);

      // First call
      await service.loadToolSchemas();
      // Force reload
      await service.loadToolSchemas(true);

      // Should invoke Tauri twice
      expect(mockInvoke).toHaveBeenCalledTimes(2);
    });

    test('should handle empty tool schema response', async () => {
      mockInvoke.mockResolvedValue([]);

      const schemas = await service.loadToolSchemas();

      expect(schemas).toHaveLength(0);
      expect(mockInvoke).toHaveBeenCalledWith('runtime_get_tool_schemas');
    });

    test('should throw error when Tauri command fails', async () => {
      const errorMessage = 'Failed to load tool schemas';
      mockInvoke.mockRejectedValue(new Error(errorMessage));

      await expect(service.loadToolSchemas()).rejects.toThrow(`Failed to load tool schemas: Error: ${errorMessage}`);
    });
  });

  describe('executeTool', () => {
    test('should execute tool using ZeroClaw format with success', async () => {
      const mockResult = {
        success: true,
        output: '{"issues": [{"title": "Bug fix", "number": 1}]}',
        error: null,
        execution_time: 1500
      };

      mockInvoke.mockResolvedValue(mockResult);

      const result = await service.executeTool(
        'github',
        'list_issues',
        '{"owner":"user","repo":"test"}'
      );

      expect(result.status).toBe('success');
      expect(result.result).toBe(mockResult.output);
      expect(result.executionTimeMs).toBe(1500);
      expect(result.toolName).toBe('list_issues');
      expect(result.skillId).toBe('github');

      // Verify correct tool_id format and arguments
      expect(mockInvoke).toHaveBeenCalledWith('runtime_execute_tool', {
        toolId: 'github_list_issues',
        arguments: '{"owner":"user","repo":"test"}'
      });
    });

    test('should handle tool execution failure', async () => {
      const mockResult = {
        success: false,
        output: '',
        error: 'Tool not found: invalid_tool',
        execution_time: 100
      };

      mockInvoke.mockResolvedValue(mockResult);

      const result = await service.executeTool(
        'invalid',
        'tool',
        '{}'
      );

      expect(result.status).toBe('error');
      expect(result.errorMessage).toBe('Tool not found: invalid_tool');
      expect(result.result).toBe('Tool not found: invalid_tool');
      expect(result.executionTimeMs).toBe(100);
    });

    test('should handle tool execution without execution_time', async () => {
      const mockResult = {
        success: true,
        output: 'Success',
        error: null
        // No execution_time provided
      };

      mockInvoke.mockResolvedValue(mockResult);

      const startTime = Date.now();
      const result = await service.executeTool('test', 'tool', '{}');
      const endTime = Date.now();

      expect(result.status).toBe('success');
      expect(result.executionTimeMs).toBeGreaterThan(0);
      expect(result.executionTimeMs).toBeLessThanOrEqual(endTime - startTime + 10); // Allow small margin
    });

    test('should handle Tauri invoke exception', async () => {
      const errorMessage = 'Network error';
      mockInvoke.mockRejectedValue(new Error(errorMessage));

      const result = await service.executeTool('test', 'tool', '{}');

      expect(result.status).toBe('error');
      expect(result.errorMessage).toBe(errorMessage);
      expect(result.result).toBe(errorMessage);
      expect(result.executionTimeMs).toBeGreaterThan(0);
    });

    test('should generate unique execution IDs', async () => {
      const mockResult = {
        success: true,
        output: 'test',
        error: null,
        execution_time: 100
      };

      mockInvoke.mockResolvedValue(mockResult);

      const result1 = await service.executeTool('test', 'tool1', '{}');
      const result2 = await service.executeTool('test', 'tool2', '{}');

      expect(result1.id).not.toBe(result2.id);
      expect(result1.id).toMatch(/^exec_\d+_[a-z0-9]+$/);
      expect(result2.id).toMatch(/^exec_\d+_[a-z0-9]+$/);
    });
  });

  describe('tool management methods', () => {
    beforeEach(async () => {
      const mockSchemas = [
        {
          type: "function",
          function: {
            name: "github_list_issues",
            description: "List GitHub issues",
            parameters: { type: "object", properties: {} }
          }
        },
        {
          type: "function",
          function: {
            name: "github_create_issue",
            description: "Create GitHub issue",
            parameters: { type: "object", properties: {} }
          }
        },
        {
          type: "function",
          function: {
            name: "notion_create_page",
            description: "Create Notion page",
            parameters: { type: "object", properties: {} }
          }
        }
      ];

      mockInvoke.mockResolvedValue(mockSchemas);
      await service.loadToolSchemas();
    });

    test('getToolByName should find tool by name', () => {
      const tool = service.getToolByName('github_list_issues');

      expect(tool).toBeDefined();
      expect(tool?.function.name).toBe('github_list_issues');
      expect(tool?.function.description).toBe('List GitHub issues');
    });

    test('getToolByName should return undefined for non-existent tool', () => {
      const tool = service.getToolByName('non_existent_tool');

      expect(tool).toBeUndefined();
    });

    test('getAllTools should return all loaded tools', () => {
      const tools = service.getAllTools();

      expect(tools).toHaveLength(3);
      expect(tools.map(t => t.function.name)).toEqual([
        'github_list_issues',
        'github_create_issue',
        'notion_create_page'
      ]);
    });

    test('getToolsBySkill should organize tools by skill ID', () => {
      const toolsBySkill = service.getToolsBySkill();

      expect(toolsBySkill).toHaveProperty('github');
      expect(toolsBySkill).toHaveProperty('notion');
      expect(toolsBySkill.github).toHaveLength(2);
      expect(toolsBySkill.notion).toHaveLength(1);

      expect(toolsBySkill.github.map(t => t.function.name)).toEqual([
        'github_list_issues',
        'github_create_issue'
      ]);
      expect(toolsBySkill.notion[0].function.name).toBe('notion_create_page');
    });

    test('getToolStats should return accurate statistics', () => {
      const stats = service.getToolStats();

      expect(stats.totalTools).toBe(3);
      expect(stats.skillCount).toBe(2);
      expect(stats.categories).toHaveProperty('GitHub', 2);
      expect(stats.categories).toHaveProperty('Notion', 1);
    });
  });

  describe('helper methods', () => {
    test('extractSkillIdFromToolName should parse skill ID correctly', () => {
      // Use reflection to access private method
      const extractMethod = (service as any).extractSkillIdFromToolName.bind(service);

      expect(extractMethod('github_list_issues')).toBe('github');
      expect(extractMethod('notion_create_page')).toBe('notion');
      expect(extractMethod('complex_skill_name_tool_name')).toBe('complex_skill_name_tool');
      expect(extractMethod('invalid_format')).toBe('invalid');
      expect(extractMethod('no_underscore')).toBeNull();
    });

    test('extractCategoryFromSkillId should categorize skills correctly', () => {
      // Use reflection to access private method
      const extractMethod = (service as any).extractCategoryFromSkillId.bind(service);

      expect(extractMethod('github')).toBe('GitHub');
      expect(extractMethod('github_enterprise')).toBe('GitHub');
      expect(extractMethod('notion')).toBe('Notion');
      expect(extractMethod('telegram')).toBe('Telegram');
      expect(extractMethod('gmail')).toBe('Email');
      expect(extractMethod('calendar')).toBe('Calendar');
      expect(extractMethod('slack')).toBe('Slack');
      expect(extractMethod('crypto_wallet')).toBe('Crypto');
      expect(extractMethod('unknown_skill')).toBe('Other');
    });
  });

  describe('clearCache', () => {
    test('should clear cached tool schemas', async () => {
      const mockSchemas = [
        {
          type: "function",
          function: {
            name: "test_tool",
            description: "Test tool",
            parameters: { type: "object", properties: {} }
          }
        }
      ];

      mockInvoke.mockResolvedValue(mockSchemas);

      // Load schemas
      await service.loadToolSchemas();
      expect(mockInvoke).toHaveBeenCalledTimes(1);

      // Clear cache
      service.clearCache();

      // Load again - should call Tauri again
      await service.loadToolSchemas();
      expect(mockInvoke).toHaveBeenCalledTimes(2);
    });
  });
});