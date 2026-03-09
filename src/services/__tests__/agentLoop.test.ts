import { describe, test, expect, beforeEach, vi, type Mock } from 'vitest';
import { AgentLoopService } from '../agentLoop';
import { AgentToolRegistry } from '../agentToolRegistry';
import { apiClient } from '../apiClient';
import type {
  AgentToolSchema,
  AgentExecutionResult,
  AgentToolExecution,
  AgentExecutionOptions
} from '../../types/agent';

// Mock dependencies
vi.mock('../agentToolRegistry');
vi.mock('../apiClient');

describe('AgentLoopService', () => {
  let service: AgentLoopService;
  const mockToolRegistry = AgentToolRegistry as vi.MockedClass<typeof AgentToolRegistry>;
  const mockApiClient = apiClient as { post: Mock };

  const mockToolSchemas: AgentToolSchema[] = [
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

  beforeEach(() => {
    service = AgentLoopService.getInstance();
    vi.clearAllMocks();

    // Setup default mock implementations
    const mockRegistryInstance = {
      loadToolSchemas: vi.fn(),
      executeTool: vi.fn()
    };

    mockToolRegistry.getInstance.mockReturnValue(mockRegistryInstance as any);
    mockRegistryInstance.loadToolSchemas.mockResolvedValue(mockToolSchemas);
  });

  describe('executeTask', () => {
    test('should execute simple task without tool calls', async () => {
      const mockResponse = {
        choices: [{
          message: {
            role: 'assistant' as const,
            content: 'Hello! How can I help you today?',
            tool_calls: undefined
          },
          finish_reason: 'stop' as const
        }],
        usage: {
          prompt_tokens: 20,
          completion_tokens: 10,
          total_tokens: 30
        }
      };

      mockApiClient.post.mockResolvedValue({ data: mockResponse });

      const result = await service.executeTask(
        'Hello',
        'conv_123',
        { maxIterations: 5, timeoutMs: 30000 }
      );

      expect(result.status).toBe('completed');
      expect(result.finalResponse).toBe('Hello! How can I help you today?');
      expect(result.iterations).toBe(1);
      expect(result.toolExecutions).toHaveLength(0);
      expect(result.executionTime).toBeGreaterThan(0);

      // Verify API call format
      expect(mockApiClient.post).toHaveBeenCalledWith(
        '/api/v1/conversations/conv_123/messages',
        expect.objectContaining({
          model: expect.any(String),
          messages: expect.arrayContaining([
            expect.objectContaining({
              role: 'user',
              content: 'Hello'
            })
          ]),
          tools: mockToolSchemas,
          tool_choice: 'auto'
        })
      );
    });

    test('should execute task with single tool call', async () => {
      const mockToolCallResponse = {
        choices: [{
          message: {
            role: 'assistant' as const,
            content: null,
            tool_calls: [{
              id: 'call_123',
              type: 'function' as const,
              function: {
                name: 'github_list_issues',
                arguments: '{"owner":"user","repo":"test"}'
              }
            }]
          },
          finish_reason: 'tool_calls' as const
        }]
      };

      const mockFinalResponse = {
        choices: [{
          message: {
            role: 'assistant' as const,
            content: 'I found 3 open issues in your repository.',
            tool_calls: undefined
          },
          finish_reason: 'stop' as const
        }],
        usage: {
          prompt_tokens: 50,
          completion_tokens: 20,
          total_tokens: 70
        }
      };

      const mockToolExecution: AgentToolExecution = {
        id: 'exec_123',
        toolName: 'list_issues',
        skillId: 'github',
        arguments: '{"owner":"user","repo":"test"}',
        status: 'success',
        startTime: Date.now() - 1500,
        endTime: Date.now(),
        executionTimeMs: 1500,
        result: '{"issues":[{"title":"Bug fix","number":1}]}'
      };

      // Setup mocks
      const mockRegistryInstance = mockToolRegistry.getInstance();
      mockRegistryInstance.executeTool.mockResolvedValue(mockToolExecution);

      mockApiClient.post
        .mockResolvedValueOnce({ data: mockToolCallResponse })
        .mockResolvedValueOnce({ data: mockFinalResponse });

      const result = await service.executeTask(
        'Show me GitHub issues',
        'conv_123'
      );

      expect(result.status).toBe('completed');
      expect(result.finalResponse).toBe('I found 3 open issues in your repository.');
      expect(result.iterations).toBe(2);
      expect(result.toolExecutions).toHaveLength(1);
      expect(result.toolExecutions[0].toolName).toBe('list_issues');
      expect(result.toolExecutions[0].status).toBe('success');

      // Verify tool execution was called with correct parameters
      expect(mockRegistryInstance.executeTool).toHaveBeenCalledWith(
        'github',
        'list_issues',
        '{"owner":"user","repo":"test"}'
      );
    });

    test('should handle multiple tool calls in sequence', async () => {
      const mockFirstToolCallResponse = {
        choices: [{
          message: {
            role: 'assistant' as const,
            content: null,
            tool_calls: [{
              id: 'call_1',
              type: 'function' as const,
              function: {
                name: 'github_list_issues',
                arguments: '{"owner":"user","repo":"test"}'
              }
            }]
          },
          finish_reason: 'tool_calls' as const
        }]
      };

      const mockSecondToolCallResponse = {
        choices: [{
          message: {
            role: 'assistant' as const,
            content: null,
            tool_calls: [{
              id: 'call_2',
              type: 'function' as const,
              function: {
                name: 'notion_create_page',
                arguments: '{"title":"Issues Summary"}'
              }
            }]
          },
          finish_reason: 'tool_calls' as const
        }]
      };

      const mockFinalResponse = {
        choices: [{
          message: {
            role: 'assistant' as const,
            content: 'I created a summary page with your GitHub issues.',
            tool_calls: undefined
          },
          finish_reason: 'stop' as const
        }]
      };

      const mockToolExecution1: AgentToolExecution = {
        id: 'exec_1',
        toolName: 'list_issues',
        skillId: 'github',
        arguments: '{"owner":"user","repo":"test"}',
        status: 'success',
        startTime: Date.now() - 2000,
        endTime: Date.now() - 1000,
        executionTimeMs: 1000,
        result: '{"issues":[{"title":"Bug fix","number":1}]}'
      };

      const mockToolExecution2: AgentToolExecution = {
        id: 'exec_2',
        toolName: 'create_page',
        skillId: 'notion',
        arguments: '{"title":"Issues Summary"}',
        status: 'success',
        startTime: Date.now() - 800,
        endTime: Date.now(),
        executionTimeMs: 800,
        result: '{"page_id":"page_123"}'
      };

      // Setup mocks
      const mockRegistryInstance = mockToolRegistry.getInstance();
      mockRegistryInstance.executeTool
        .mockResolvedValueOnce(mockToolExecution1)
        .mockResolvedValueOnce(mockToolExecution2);

      mockApiClient.post
        .mockResolvedValueOnce({ data: mockFirstToolCallResponse })
        .mockResolvedValueOnce({ data: mockSecondToolCallResponse })
        .mockResolvedValueOnce({ data: mockFinalResponse });

      const result = await service.executeTask(
        'Get GitHub issues and create a summary page',
        'conv_123',
        { maxIterations: 5 }
      );

      expect(result.status).toBe('completed');
      expect(result.iterations).toBe(3);
      expect(result.toolExecutions).toHaveLength(2);
      expect(result.toolExecutions[0].skillId).toBe('github');
      expect(result.toolExecutions[1].skillId).toBe('notion');
    });

    test('should handle tool execution timeout', async () => {
      const result = await service.executeTask(
        'Test timeout',
        'conv_123',
        { maxIterations: 1, timeoutMs: 100 } // Very short timeout
      );

      // The timeout logic depends on how it's implemented in the actual service
      // This test may need adjustment based on the actual implementation
      expect(result.status).toBe('timeout');
      expect(result.error).toContain('timeout');
    });

    test('should respect maximum iterations limit', async () => {
      const mockToolCallResponse = {
        choices: [{
          message: {
            role: 'assistant' as const,
            tool_calls: [{
              id: 'call_1',
              type: 'function' as const,
              function: { name: 'github_list_issues', arguments: '{}' }
            }]
          },
          finish_reason: 'tool_calls' as const
        }]
      };

      // Mock to always return tool calls (infinite loop scenario)
      mockApiClient.post.mockResolvedValue({ data: mockToolCallResponse });

      const mockToolExecution: AgentToolExecution = {
        id: 'exec_1',
        toolName: 'list_issues',
        skillId: 'github',
        arguments: '{}',
        status: 'success',
        startTime: Date.now() - 100,
        endTime: Date.now(),
        executionTimeMs: 100,
        result: '{}'
      };

      const mockRegistryInstance = mockToolRegistry.getInstance();
      mockRegistryInstance.executeTool.mockResolvedValue(mockToolExecution);

      const result = await service.executeTask(
        'Infinite loop test',
        'conv_123',
        { maxIterations: 2, timeoutMs: 10000 }
      );

      expect(result.status).toBe('max_iterations');
      expect(result.iterations).toBe(2);
      expect(result.error).toContain('maximum iterations');
    });

    test('should handle tool execution error gracefully', async () => {
      const mockToolCallResponse = {
        choices: [{
          message: {
            role: 'assistant' as const,
            tool_calls: [{
              id: 'call_1',
              type: 'function' as const,
              function: {
                name: 'invalid_tool',
                arguments: '{}'
              }
            }]
          },
          finish_reason: 'tool_calls' as const
        }]
      };

      const mockErrorResponse = {
        choices: [{
          message: {
            role: 'assistant' as const,
            content: 'I encountered an error while executing the tool.',
            tool_calls: undefined
          },
          finish_reason: 'stop' as const
        }]
      };

      const mockToolExecution: AgentToolExecution = {
        id: 'exec_1',
        toolName: 'invalid_tool',
        skillId: 'unknown',
        arguments: '{}',
        status: 'error',
        startTime: Date.now() - 100,
        endTime: Date.now(),
        executionTimeMs: 100,
        errorMessage: 'Tool not found'
      };

      const mockRegistryInstance = mockToolRegistry.getInstance();
      mockRegistryInstance.executeTool.mockResolvedValue(mockToolExecution);

      mockApiClient.post
        .mockResolvedValueOnce({ data: mockToolCallResponse })
        .mockResolvedValueOnce({ data: mockErrorResponse });

      const result = await service.executeTask(
        'Test error handling',
        'conv_123'
      );

      expect(result.status).toBe('completed');
      expect(result.toolExecutions).toHaveLength(1);
      expect(result.toolExecutions[0].status).toBe('error');
      expect(result.toolExecutions[0].errorMessage).toBe('Tool not found');
    });

    test('should handle API client errors', async () => {
      mockApiClient.post.mockRejectedValue(new Error('Network error'));

      const result = await service.executeTask(
        'Test API error',
        'conv_123'
      );

      expect(result.status).toBe('error');
      expect(result.error).toContain('Network error');
      expect(result.iterations).toBe(0);
      expect(result.toolExecutions).toHaveLength(0);
    });

    test('should parse tool name from function name correctly', async () => {
      const mockToolCallResponse = {
        choices: [{
          message: {
            role: 'assistant' as const,
            tool_calls: [{
              id: 'call_1',
              type: 'function' as const,
              function: {
                name: 'github_list_issues', // Should parse to skillId=github, toolName=list_issues
                arguments: '{"owner":"user","repo":"test"}'
              }
            }]
          },
          finish_reason: 'tool_calls' as const
        }]
      };

      const mockFinalResponse = {
        choices: [{
          message: {
            role: 'assistant' as const,
            content: 'Done',
            tool_calls: undefined
          },
          finish_reason: 'stop' as const
        }]
      };

      const mockToolExecution: AgentToolExecution = {
        id: 'exec_1',
        toolName: 'list_issues',
        skillId: 'github',
        arguments: '{"owner":"user","repo":"test"}',
        status: 'success',
        startTime: Date.now() - 100,
        endTime: Date.now(),
        executionTimeMs: 100,
        result: '{}'
      };

      const mockRegistryInstance = mockToolRegistry.getInstance();
      mockRegistryInstance.executeTool.mockResolvedValue(mockToolExecution);

      mockApiClient.post
        .mockResolvedValueOnce({ data: mockToolCallResponse })
        .mockResolvedValueOnce({ data: mockFinalResponse });

      await service.executeTask('Test tool parsing', 'conv_123');

      // Verify correct parsing of skill ID and tool name
      expect(mockRegistryInstance.executeTool).toHaveBeenCalledWith(
        'github',
        'list_issues',
        '{"owner":"user","repo":"test"}'
      );
    });
  });

  describe('singleton behavior', () => {
    test('should return the same instance', () => {
      const instance1 = AgentLoopService.getInstance();
      const instance2 = AgentLoopService.getInstance();

      expect(instance1).toBe(instance2);
    });
  });

  describe('task execution options', () => {
    test('should use default options when none provided', async () => {
      const mockResponse = {
        choices: [{
          message: {
            role: 'assistant' as const,
            content: 'Test response'
          },
          finish_reason: 'stop' as const
        }]
      };

      mockApiClient.post.mockResolvedValue({ data: mockResponse });

      const result = await service.executeTask('Test', 'conv_123');

      // Should complete successfully with defaults
      expect(result.status).toBe('completed');
      expect(result.executionTime).toBeGreaterThan(0);
    });

    test('should respect custom execution options', async () => {
      const mockResponse = {
        choices: [{
          message: {
            role: 'assistant' as const,
            content: 'Test response'
          },
          finish_reason: 'stop' as const
        }]
      };

      mockApiClient.post.mockResolvedValue({ data: mockResponse });

      const customOptions: AgentExecutionOptions = {
        maxIterations: 3,
        timeoutMs: 5000,
        model: 'gpt-3.5-turbo',
        temperature: 0.7
      };

      const result = await service.executeTask('Test', 'conv_123', customOptions);

      expect(result.status).toBe('completed');

      // Verify custom options were passed to API
      expect(mockApiClient.post).toHaveBeenCalledWith(
        '/api/v1/conversations/conv_123/messages',
        expect.objectContaining({
          model: 'gpt-3.5-turbo',
          temperature: 0.7
        })
      );
    });
  });
});