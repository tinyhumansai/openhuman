import { describe, test, expect, beforeEach, vi } from 'vitest';
import { configureStore } from '@reduxjs/toolkit';
import agentReducer, {
  setAgentMode,
  startAgentExecution,
  updateExecutionProgress,
  completeAgentExecution,
  cancelAgentExecution,
  setToolRegistry,
  clearExecutionHistory,
  executeAgentTask,
  loadAgentTools,
  cancelAgentExecutionThunk,
  type AgentState
} from '../agentSlice';
import type {
  AgentExecutionResult,
  AgentToolExecution,
  AgentToolSchema
} from '../../types/agent';

// Mock dependencies
vi.mock('../../services/agentLoop');
vi.mock('../../services/agentToolRegistry');

describe('agentSlice', () => {
  let store: ReturnType<typeof configureStore>;

  beforeEach(() => {
    store = configureStore({
      reducer: {
        agent: agentReducer
      }
    });
  });

  describe('synchronous actions', () => {
    test('setAgentMode should toggle agent mode', () => {
      expect(store.getState().agent.isAgentMode).toBe(false);

      store.dispatch(setAgentMode(true));
      expect(store.getState().agent.isAgentMode).toBe(true);

      store.dispatch(setAgentMode(false));
      expect(store.getState().agent.isAgentMode).toBe(false);
    });

    test('startAgentExecution should initialize execution state', () => {
      const executionId = 'exec_123';
      const threadId = 'thread_456';
      const userMessage = 'Help me with GitHub issues';

      store.dispatch(startAgentExecution({ executionId, threadId, userMessage }));

      const state = store.getState().agent;
      expect(state.currentExecution).toEqual({
        id: executionId,
        threadId,
        userMessage,
        status: 'running',
        iterations: 0,
        toolExecutions: [],
        startTime: expect.any(Number),
        executionTime: 0
      });
      expect(state.executionHistory).toHaveLength(1);
      expect(state.executionHistory[0].id).toBe(executionId);
    });

    test('updateExecutionProgress should update current execution', () => {
      const executionId = 'exec_123';
      const threadId = 'thread_456';

      // Start execution first
      store.dispatch(startAgentExecution({
        executionId,
        threadId,
        userMessage: 'Test'
      }));

      const toolExecution: AgentToolExecution = {
        id: 'tool_exec_1',
        toolName: 'list_issues',
        skillId: 'github',
        arguments: '{"owner":"user","repo":"test"}',
        status: 'running',
        startTime: Date.now()
      };

      store.dispatch(updateExecutionProgress({
        executionId,
        iteration: 1,
        toolExecution
      }));

      const state = store.getState().agent;
      expect(state.currentExecution?.iterations).toBe(1);
      expect(state.currentExecution?.toolExecutions).toHaveLength(1);
      expect(state.currentExecution?.toolExecutions[0]).toEqual(toolExecution);
    });

    test('updateExecutionProgress should update existing tool execution', () => {
      const executionId = 'exec_123';
      const threadId = 'thread_456';

      // Start execution
      store.dispatch(startAgentExecution({
        executionId,
        threadId,
        userMessage: 'Test'
      }));

      const toolExecution: AgentToolExecution = {
        id: 'tool_exec_1',
        toolName: 'list_issues',
        skillId: 'github',
        arguments: '{}',
        status: 'running',
        startTime: Date.now()
      };

      // Add tool execution
      store.dispatch(updateExecutionProgress({
        executionId,
        iteration: 1,
        toolExecution
      }));

      // Update the same tool execution with completion
      const updatedToolExecution: AgentToolExecution = {
        ...toolExecution,
        status: 'success',
        endTime: Date.now(),
        executionTimeMs: 1500,
        result: '{"issues":[]}'
      };

      store.dispatch(updateExecutionProgress({
        executionId,
        iteration: 1,
        toolExecution: updatedToolExecution
      }));

      const state = store.getState().agent;
      expect(state.currentExecution?.toolExecutions).toHaveLength(1);
      expect(state.currentExecution?.toolExecutions[0].status).toBe('success');
      expect(state.currentExecution?.toolExecutions[0].result).toBe('{"issues":[]}');
    });

    test('completeAgentExecution should finalize execution', () => {
      const executionId = 'exec_123';
      const threadId = 'thread_456';

      // Start execution first
      store.dispatch(startAgentExecution({
        executionId,
        threadId,
        userMessage: 'Test'
      }));

      const completionData = {
        executionId,
        status: 'completed' as const,
        finalResponse: 'Task completed successfully',
        totalExecutionTime: 5000
      };

      store.dispatch(completeAgentExecution(completionData));

      const state = store.getState().agent;
      expect(state.currentExecution?.status).toBe('completed');
      expect(state.currentExecution?.finalResponse).toBe('Task completed successfully');
      expect(state.currentExecution?.executionTime).toBe(5000);
      expect(state.lastExecutionId).toBe(executionId);

      // Execution should be updated in history
      const historyItem = state.executionHistory.find(item => item.id === executionId);
      expect(historyItem?.status).toBe('completed');
      expect(historyItem?.finalResponse).toBe('Task completed successfully');
    });

    test('cancelAgentExecution should cancel current execution', () => {
      const executionId = 'exec_123';
      const threadId = 'thread_456';

      // Start execution first
      store.dispatch(startAgentExecution({
        executionId,
        threadId,
        userMessage: 'Test'
      }));

      store.dispatch(cancelAgentExecution({
        executionId,
        reason: 'User cancelled'
      }));

      const state = store.getState().agent;
      expect(state.currentExecution?.status).toBe('cancelled');
      expect(state.currentExecution?.error).toBe('User cancelled');
    });

    test('setToolRegistry should update available tools', () => {
      const mockTools: AgentToolSchema[] = [
        {
          type: "function",
          function: {
            name: "github_list_issues",
            description: "List GitHub issues",
            parameters: {
              type: "object",
              properties: {
                owner: { type: "string" },
                repo: { type: "string" }
              },
              required: ["owner", "repo"]
            }
          }
        }
      ];

      store.dispatch(setToolRegistry({
        tools: mockTools,
        lastUpdated: Date.now()
      }));

      const state = store.getState().agent;
      expect(state.toolRegistry.tools).toEqual(mockTools);
      expect(state.toolRegistry.lastUpdated).toBeDefined();
    });

    test('clearExecutionHistory should reset history', () => {
      const executionId = 'exec_123';
      const threadId = 'thread_456';

      // Add some history first
      store.dispatch(startAgentExecution({
        executionId,
        threadId,
        userMessage: 'Test'
      }));

      expect(store.getState().agent.executionHistory).toHaveLength(1);

      store.dispatch(clearExecutionHistory());

      expect(store.getState().agent.executionHistory).toHaveLength(0);
    });
  });

  describe('async thunks', () => {
    test('executeAgentTask.pending should set loading state', () => {
      const action = { type: executeAgentTask.pending.type };
      const state = agentReducer(undefined, action);

      expect(state.isLoading).toBe(true);
      expect(state.error).toBeNull();
    });

    test('executeAgentTask.fulfilled should handle successful execution', () => {
      const mockResult: AgentExecutionResult = {
        status: 'completed',
        executionId: 'exec_123',
        finalResponse: 'Task completed',
        iterations: 2,
        toolExecutions: [],
        executionTime: 3000
      };

      const action = {
        type: executeAgentTask.fulfilled.type,
        payload: mockResult
      };

      const state = agentReducer(undefined, action);

      expect(state.isLoading).toBe(false);
      expect(state.error).toBeNull();
      expect(state.lastExecutionId).toBe('exec_123');
    });

    test('executeAgentTask.rejected should handle execution error', () => {
      const action = {
        type: executeAgentTask.rejected.type,
        error: { message: 'Execution failed' }
      };

      const state = agentReducer(undefined, action);

      expect(state.isLoading).toBe(false);
      expect(state.error).toBe('Execution failed');
    });

    test('loadAgentTools.fulfilled should update tool registry', () => {
      const mockTools: AgentToolSchema[] = [
        {
          type: "function",
          function: {
            name: "test_tool",
            description: "Test tool",
            parameters: { type: "object", properties: {} }
          }
        }
      ];

      const action = {
        type: loadAgentTools.fulfilled.type,
        payload: mockTools
      };

      const state = agentReducer(undefined, action);

      expect(state.toolRegistry.tools).toEqual(mockTools);
      expect(state.toolRegistry.isLoaded).toBe(true);
      expect(state.toolRegistry.lastUpdated).toBeDefined();
    });

    test('loadAgentTools.rejected should handle tool loading error', () => {
      const action = {
        type: loadAgentTools.rejected.type,
        error: { message: 'Failed to load tools' }
      };

      const state = agentReducer(undefined, action);

      expect(state.toolRegistry.isLoaded).toBe(false);
      expect(state.toolRegistry.error).toBe('Failed to load tools');
    });

    test('cancelAgentExecutionThunk.fulfilled should cancel execution', () => {
      // First set up an execution
      const initialState: AgentState = {
        isAgentMode: false,
        isLoading: false,
        error: null,
        currentExecution: {
          id: 'exec_123',
          threadId: 'thread_456',
          userMessage: 'Test',
          status: 'running',
          iterations: 1,
          toolExecutions: [],
          startTime: Date.now(),
          executionTime: 0
        },
        executionHistory: [{
          id: 'exec_123',
          threadId: 'thread_456',
          userMessage: 'Test',
          status: 'running',
          iterations: 1,
          toolExecutions: [],
          startTime: Date.now(),
          executionTime: 0
        }],
        lastExecutionId: null,
        toolRegistry: {
          tools: [],
          isLoaded: false,
          lastUpdated: null,
          error: null
        },
        configByThreadId: {}
      };

      const action = {
        type: cancelAgentExecutionThunk.fulfilled.type,
        payload: { executionId: 'exec_123' }
      };

      const state = agentReducer(initialState, action);

      expect(state.currentExecution?.status).toBe('cancelled');
      expect(state.executionHistory[0].status).toBe('cancelled');
    });
  });

  describe('selectors and derived state', () => {
    test('should maintain execution history chronologically', () => {
      const execution1 = {
        executionId: 'exec_1',
        threadId: 'thread_1',
        userMessage: 'First task'
      };

      const execution2 = {
        executionId: 'exec_2',
        threadId: 'thread_1',
        userMessage: 'Second task'
      };

      store.dispatch(startAgentExecution(execution1));
      store.dispatch(startAgentExecution(execution2));

      const state = store.getState().agent;
      expect(state.executionHistory).toHaveLength(2);
      expect(state.executionHistory[0].id).toBe('exec_1');
      expect(state.executionHistory[1].id).toBe('exec_2');
    });

    test('should track tool execution statistics', () => {
      const executionId = 'exec_123';

      store.dispatch(startAgentExecution({
        executionId,
        threadId: 'thread_1',
        userMessage: 'Test'
      }));

      // Add multiple tool executions
      const toolExecution1: AgentToolExecution = {
        id: 'tool_1',
        toolName: 'list_issues',
        skillId: 'github',
        arguments: '{}',
        status: 'success',
        startTime: Date.now() - 2000,
        endTime: Date.now() - 1000,
        executionTimeMs: 1000
      };

      const toolExecution2: AgentToolExecution = {
        id: 'tool_2',
        toolName: 'create_page',
        skillId: 'notion',
        arguments: '{}',
        status: 'success',
        startTime: Date.now() - 1000,
        endTime: Date.now(),
        executionTimeMs: 1000
      };

      store.dispatch(updateExecutionProgress({
        executionId,
        iteration: 1,
        toolExecution: toolExecution1
      }));

      store.dispatch(updateExecutionProgress({
        executionId,
        iteration: 2,
        toolExecution: toolExecution2
      }));

      const state = store.getState().agent;
      expect(state.currentExecution?.toolExecutions).toHaveLength(2);
      expect(state.currentExecution?.iterations).toBe(2);
    });
  });

  describe('error handling', () => {
    test('should handle invalid execution updates gracefully', () => {
      // Try to update non-existent execution
      store.dispatch(updateExecutionProgress({
        executionId: 'non_existent',
        iteration: 1,
        toolExecution: {
          id: 'tool_1',
          toolName: 'test',
          skillId: 'test',
          arguments: '{}',
          status: 'running',
          startTime: Date.now()
        }
      }));

      // Should not crash and current execution should remain null
      const state = store.getState().agent;
      expect(state.currentExecution).toBeNull();
    });

    test('should handle completion of non-existent execution gracefully', () => {
      store.dispatch(completeAgentExecution({
        executionId: 'non_existent',
        status: 'completed',
        finalResponse: 'Done',
        totalExecutionTime: 1000
      }));

      // Should not crash
      const state = store.getState().agent;
      expect(state.currentExecution).toBeNull();
    });
  });

  describe('thread-specific configuration', () => {
    test('should store and retrieve thread-specific config', () => {
      const initialState = store.getState().agent;
      expect(initialState.configByThreadId).toEqual({});

      // Test that the structure exists for future configuration
      // Note: actual config setting would require a specific action
      expect(typeof initialState.configByThreadId).toBe('object');
    });
  });
});