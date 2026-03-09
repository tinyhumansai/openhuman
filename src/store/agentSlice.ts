/**
 * Agent Redux slice for managing agent execution state
 *
 * Extends AlphaHuman's existing Redux pattern to handle agent task execution,
 * tool executions, and agent configuration per thread.
 */

import { createSlice, createAsyncThunk, type PayloadAction } from '@reduxjs/toolkit';

import { AgentLoop } from '../services/agentLoop';
import { AgentToolRegistry } from '../services/agentToolRegistry';
import type {
  AgentState,
  AgentExecution,
  AgentExecutionResult,
  AgentExecutionOptions,
  AgentExecutionHistoryEntry,
  AgentToolExecution,
  AgentToolSchema
} from '../types/agent';

// =============================================================================
// Async Thunks
// =============================================================================

/**
 * Execute an agent task autonomously
 */
export const executeAgentTask = createAsyncThunk(
  'agent/executeTask',
  async (
    params: {
      userMessage: string;
      threadId: string;
      options?: AgentExecutionOptions;
    },
    { getState, rejectWithValue }
  ) => {
    try {
      const agentLoop = AgentLoop.getInstance();
      const result = await agentLoop.executeTask(
        params.userMessage,
        params.threadId,
        params.options
      );

      return {
        threadId: params.threadId,
        userMessage: params.userMessage,
        result,
        timestamp: Date.now()
      };
    } catch (error) {
      return rejectWithValue(
        error instanceof Error ? error.message : String(error)
      );
    }
  }
);

/**
 * Load available tools from the skill system
 */
export const loadAgentTools = createAsyncThunk(
  'agent/loadTools',
  async (forceReload = false, { rejectWithValue }) => {
    try {
      const registry = AgentToolRegistry.getInstance();
      const tools = await registry.loadToolSchemas(forceReload);
      return tools;
    } catch (error) {
      return rejectWithValue(
        error instanceof Error ? error.message : String(error)
      );
    }
  }
);

/**
 * Cancel an active agent execution
 */
export const cancelAgentExecution = createAsyncThunk(
  'agent/cancelExecution',
  async (executionId: string, { rejectWithValue }) => {
    try {
      const agentLoop = AgentLoop.getInstance();
      const cancelled = agentLoop.cancelExecution(executionId);

      if (!cancelled) {
        throw new Error('Execution not found or already completed');
      }

      return executionId;
    } catch (error) {
      return rejectWithValue(
        error instanceof Error ? error.message : String(error)
      );
    }
  }
);

// =============================================================================
// Initial State
// =============================================================================

const initialState: AgentState = {
  agentModeByThreadId: {},
  activeExecutions: {},
  executionHistory: [],
  configByThreadId: {},
  toolRegistry: {
    tools: [],
    lastUpdated: 0,
    loading: false
  },
  ui: {
    showExecutionDetails: {},
    selectedExecution: undefined
  }
};

// =============================================================================
// Slice Definition
// =============================================================================

const agentSlice = createSlice({
  name: 'agent',
  initialState,
  reducers: {
    // Agent mode management
    setAgentModeForThread: (
      state,
      action: PayloadAction<{ threadId: string; enabled: boolean }>
    ) => {
      const { threadId, enabled } = action.payload;
      state.agentModeByThreadId[threadId] = enabled;
    },

    // Agent configuration management
    setAgentConfigForThread: (
      state,
      action: PayloadAction<{ threadId: string; config: AgentExecutionOptions }>
    ) => {
      const { threadId, config } = action.payload;
      state.configByThreadId[threadId] = config;
    },

    // UI state management
    toggleExecutionDetails: (
      state,
      action: PayloadAction<{ executionId: string }>
    ) => {
      const { executionId } = action.payload;
      state.ui.showExecutionDetails[executionId] = !state.ui.showExecutionDetails[executionId];
    },

    setSelectedExecution: (
      state,
      action: PayloadAction<string | undefined>
    ) => {
      state.ui.selectedExecution = action.payload;
    },

    // Tool registry cache management
    clearToolRegistry: (state) => {
      state.toolRegistry = {
        tools: [],
        lastUpdated: 0,
        loading: false
      };
    },

    // Execution tracking (for real-time updates)
    addActiveExecution: (
      state,
      action: PayloadAction<AgentExecution>
    ) => {
      const execution = action.payload;
      state.activeExecutions[execution.id] = execution;
    },

    updateActiveExecution: (
      state,
      action: PayloadAction<Partial<AgentExecution> & { id: string }>
    ) => {
      const { id, ...updates } = action.payload;
      if (state.activeExecutions[id]) {
        Object.assign(state.activeExecutions[id], updates);
      }
    },

    removeActiveExecution: (
      state,
      action: PayloadAction<string>
    ) => {
      const executionId = action.payload;
      delete state.activeExecutions[executionId];
    },

    // Tool execution updates
    addToolExecution: (
      state,
      action: PayloadAction<{ executionId: string; toolExecution: AgentToolExecution }>
    ) => {
      const { executionId, toolExecution } = action.payload;
      if (state.activeExecutions[executionId]) {
        state.activeExecutions[executionId].toolExecutions.push(toolExecution);
        state.activeExecutions[executionId].lastUpdate = Date.now();
      }
    },

    updateToolExecution: (
      state,
      action: PayloadAction<{
        executionId: string;
        toolExecutionId: string;
        updates: Partial<AgentToolExecution>;
      }>
    ) => {
      const { executionId, toolExecutionId, updates } = action.payload;
      const execution = state.activeExecutions[executionId];

      if (execution) {
        const toolExecution = execution.toolExecutions.find(te => te.id === toolExecutionId);
        if (toolExecution) {
          Object.assign(toolExecution, updates);
          execution.lastUpdate = Date.now();
        }
      }
    },

    // Execution history management
    addExecutionToHistory: (
      state,
      action: PayloadAction<AgentExecutionHistoryEntry>
    ) => {
      state.executionHistory.unshift(action.payload);

      // Keep only last 100 executions
      if (state.executionHistory.length > 100) {
        state.executionHistory = state.executionHistory.slice(0, 100);
      }
    },

    clearExecutionHistory: (state) => {
      state.executionHistory = [];
    }
  },

  extraReducers: (builder) => {
    // Execute agent task
    builder
      .addCase(executeAgentTask.pending, (state, action) => {
        const { userMessage, threadId } = action.meta.arg;
        const executionId = `agent_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;

        const execution: AgentExecution = {
          id: executionId,
          threadId,
          userMessage,
          status: 'initializing',
          currentIteration: 0,
          maxIterations: action.meta.arg.options?.maxIterations || 10,
          toolExecutions: [],
          startTime: Date.now(),
          lastUpdate: Date.now()
        };

        state.activeExecutions[executionId] = execution;
      })
      .addCase(executeAgentTask.fulfilled, (state, action) => {
        const { threadId, userMessage, result, timestamp } = action.payload;

        // Find the execution by thread and message
        const execution = Object.values(state.activeExecutions).find(
          exec => exec.threadId === threadId && exec.userMessage === userMessage
        );

        if (execution) {
          // Move from active to history
          const historyEntry: AgentExecutionHistoryEntry = {
            executionId: execution.id,
            threadId,
            userMessage,
            result,
            timestamp,
            duration: Date.now() - execution.startTime
          };

          state.executionHistory.unshift(historyEntry);
          delete state.activeExecutions[execution.id];

          // Keep only last 100 executions
          if (state.executionHistory.length > 100) {
            state.executionHistory = state.executionHistory.slice(0, 100);
          }
        }
      })
      .addCase(executeAgentTask.rejected, (state, action) => {
        // Remove failed execution from active list
        const rejectedExecution = Object.values(state.activeExecutions).find(
          exec => exec.userMessage === action.meta.arg.userMessage
        );

        if (rejectedExecution) {
          delete state.activeExecutions[rejectedExecution.id];
        }
      });

    // Load agent tools
    builder
      .addCase(loadAgentTools.pending, (state) => {
        state.toolRegistry.loading = true;
      })
      .addCase(loadAgentTools.fulfilled, (state, action) => {
        state.toolRegistry.tools = action.payload;
        state.toolRegistry.lastUpdated = Date.now();
        state.toolRegistry.loading = false;
        state.toolRegistry.error = undefined;
      })
      .addCase(loadAgentTools.rejected, (state, action) => {
        state.toolRegistry.loading = false;
        state.toolRegistry.error = action.payload as string;
      });

    // Cancel agent execution
    builder
      .addCase(cancelAgentExecution.fulfilled, (state, action) => {
        const executionId = action.payload;
        if (state.activeExecutions[executionId]) {
          state.activeExecutions[executionId].status = 'completing';
        }
      });
  }
});

// =============================================================================
// Actions Export
// =============================================================================

export const {
  setAgentModeForThread,
  setAgentConfigForThread,
  toggleExecutionDetails,
  setSelectedExecution,
  clearToolRegistry,
  addActiveExecution,
  updateActiveExecution,
  removeActiveExecution,
  addToolExecution,
  updateToolExecution,
  addExecutionToHistory,
  clearExecutionHistory
} = agentSlice.actions;

// =============================================================================
// Selectors
// =============================================================================

export const selectAgentModeForThread = (state: { agent: AgentState }, threadId: string) =>
  state.agent.agentModeByThreadId[threadId] || false;

export const selectAgentConfigForThread = (state: { agent: AgentState }, threadId: string) =>
  state.agent.configByThreadId[threadId] || {};

export const selectActiveExecutions = (state: { agent: AgentState }) =>
  Object.values(state.agent.activeExecutions);

export const selectActiveExecutionForThread = (state: { agent: AgentState }, threadId: string) =>
  Object.values(state.agent.activeExecutions).find(exec => exec.threadId === threadId);

export const selectExecutionHistory = (state: { agent: AgentState }) =>
  state.agent.executionHistory;

export const selectExecutionHistoryForThread = (state: { agent: AgentState }, threadId: string) =>
  state.agent.executionHistory.filter(entry => entry.threadId === threadId);

export const selectToolRegistry = (state: { agent: AgentState }) =>
  state.agent.toolRegistry;

export const selectAvailableTools = (state: { agent: AgentState }) =>
  state.agent.toolRegistry.tools;

export const selectToolsByCategory = (state: { agent: AgentState }) => {
  const toolsBySkill: Record<string, AgentToolSchema[]> = {};

  for (const tool of state.agent.toolRegistry.tools) {
    const skillId = (tool.function as any).skillId || 'unknown';
    if (!toolsBySkill[skillId]) {
      toolsBySkill[skillId] = [];
    }
    toolsBySkill[skillId].push(tool);
  }

  return toolsBySkill;
};

export const selectToolStats = (state: { agent: AgentState }) => {
  const tools = state.agent.toolRegistry.tools;
  const skillIds = new Set<string>();
  const categories: Record<string, number> = {};

  for (const tool of tools) {
    const skillId = (tool.function as any).skillId || 'unknown';
    skillIds.add(skillId);

    // Categorize by skill type
    let category = 'Other';
    if (skillId.includes('github') || skillId.includes('git')) category = 'GitHub';
    else if (skillId.includes('notion')) category = 'Notion';
    else if (skillId.includes('telegram') || skillId.includes('tg')) category = 'Telegram';
    else if (skillId.includes('email') || skillId.includes('gmail')) category = 'Email';
    else if (skillId.includes('calendar')) category = 'Calendar';
    else if (skillId.includes('slack')) category = 'Slack';

    categories[category] = (categories[category] || 0) + 1;
  }

  return {
    totalTools: tools.length,
    skillCount: skillIds.size,
    categories
  };
};

export const selectAgentUIState = (state: { agent: AgentState }) =>
  state.agent.ui;

// =============================================================================
// Reducer Export
// =============================================================================

export default agentSlice.reducer;