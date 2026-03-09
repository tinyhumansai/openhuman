/**
 * Agent Loop Service
 *
 * Orchestrates autonomous agent task execution by:
 * 1. Loading tools from the existing skill system
 * 2. Sending requests to the backend (which proxies to AI providers)
 * 3. Executing tool calls using the skill system
 * 4. Managing conversation state and iteration
 */

import { AgentToolRegistry } from './agentToolRegistry';
import { apiClient } from './apiClient';
import type {
  AgentExecutionOptions,
  AgentExecutionResult,
  AgentToolExecution,
  AgentChatRequest,
  AgentChatResponse,
  OpenAIMessage,
  OpenAITool,
  IAgentLoop
} from '../types/agent';

export class AgentLoop implements IAgentLoop {
  private static instance: AgentLoop;
  private toolRegistry: AgentToolRegistry;
  private activeExecutions = new Map<string, AbortController>();

  constructor() {
    this.toolRegistry = AgentToolRegistry.getInstance();
  }

  static getInstance(): AgentLoop {
    if (!this.instance) {
      this.instance = new AgentLoop();
    }
    return this.instance;
  }

  /**
   * Execute an agent task autonomously
   */
  async executeTask(
    userMessage: string,
    threadId: string,
    options: AgentExecutionOptions = {}
  ): Promise<AgentExecutionResult> {
    const {
      maxIterations = 10,
      timeoutMs = 300000, // 5 minutes
      requireApproval = false,
      allowedSkills,
      blockedTools = [],
      retryFailedTools = false
    } = options;

    const executionId = `agent_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;
    const abortController = new AbortController();
    this.activeExecutions.set(executionId, abortController);

    const startTime = Date.now();
    const toolExecutions: AgentToolExecution[] = [];
    let iterations = 0;

    try {
      console.log(`🤖 Starting agent task execution (${executionId})`);
      console.log(`📝 User message: "${userMessage}"`);
      console.log(`⚙️ Options:`, { maxIterations, timeoutMs, allowedSkills, blockedTools });

      // Set up timeout
      const timeoutId = setTimeout(() => {
        console.log(`⏰ Agent execution timeout (${timeoutMs}ms)`);
        abortController.abort();
      }, timeoutMs);

      try {
        // Load available tools from skill system
        console.log('🔧 Loading available tools from skills...');
        const toolSchemas = await this.toolRegistry.loadToolSchemas();

        // Filter tools based on configuration
        const availableTools = this.filterTools(toolSchemas, allowedSkills, blockedTools);
        console.log(`🛠️ Agent has access to ${availableTools.length} tools from ${toolSchemas.length} total`);

        // Convert to OpenAI format for backend compatibility
        const tools = availableTools.map(this.convertToOpenAITool);

        // Initialize conversation with user message
        const messages: OpenAIMessage[] = [
          {
            role: 'user',
            content: userMessage
          }
        ];

        let finalResponse: string | undefined;

        // Agent iteration loop
        while (iterations < maxIterations && !abortController.signal.aborted) {
          iterations++;
          console.log(`🔄 Agent iteration ${iterations}/${maxIterations}`);

          try {
            // Send request to backend (which proxies to AI provider)
            const request: AgentChatRequest = {
              model: 'gpt-4', // Backend will handle the actual model
              messages: [...messages],
              tools,
              tool_choice: 'auto',
              temperature: 0.7,
              max_tokens: 4096
            };

            console.log('📤 Sending request to backend proxy...');
            const response = await apiClient.post<AgentChatResponse>(
              `/api/v1/conversations/${threadId}/messages`,
              request,
              {
                signal: abortController.signal
              }
            );

            const assistantMessage = response.data.choices[0]?.message;
            if (!assistantMessage) {
              throw new Error('No response from AI provider');
            }

            console.log(`📥 Received response: ${assistantMessage.tool_calls?.length || 0} tool calls`);

            // Add assistant message to conversation
            messages.push(assistantMessage);

            // Check if AI wants to call tools
            if (assistantMessage.tool_calls && assistantMessage.tool_calls.length > 0) {
              console.log(`🛠️ Executing ${assistantMessage.tool_calls.length} tool calls...`);

              // Execute each tool call
              for (const toolCall of assistantMessage.tool_calls) {
                if (abortController.signal.aborted) {
                  break;
                }

                const execution = await this.executeSingleTool(
                  toolCall,
                  availableTools,
                  requireApproval,
                  abortController.signal
                );

                toolExecutions.push(execution);

                // Add tool result to conversation
                messages.push({
                  role: 'tool',
                  content: execution.result || execution.errorMessage || 'No result',
                  tool_call_id: toolCall.id
                });

                console.log(`✅ Tool result added to conversation: ${execution.status}`);
              }

              // Continue to next iteration to let AI process tool results
              continue;
            } else {
              // AI provided final response
              finalResponse = assistantMessage.content || '';
              console.log('✅ Agent task completed with final response');
              break;
            }

          } catch (error) {
            console.error(`❌ Error in agent iteration ${iterations}:`, error);

            if (abortController.signal.aborted) {
              clearTimeout(timeoutId);
              return {
                status: 'timeout',
                executionId,
                iterations,
                toolExecutions,
                executionTime: Date.now() - startTime,
                error: 'Execution timed out'
              };
            }

            clearTimeout(timeoutId);
            return {
              status: 'error',
              executionId,
              iterations,
              toolExecutions,
              executionTime: Date.now() - startTime,
              error: error instanceof Error ? error.message : String(error)
            };
          }
        }

        clearTimeout(timeoutId);

        // Check if we hit max iterations
        if (iterations >= maxIterations && !finalResponse) {
          console.log('⚠️ Agent reached maximum iterations without completion');
          return {
            status: 'max_iterations',
            executionId,
            iterations,
            toolExecutions,
            executionTime: Date.now() - startTime,
            error: 'Maximum iterations reached without completion'
          };
        }

        const executionTime = Date.now() - startTime;
        console.log(`🎉 Agent execution completed successfully in ${executionTime}ms`);
        console.log(`📊 Stats: ${iterations} iterations, ${toolExecutions.length} tool executions`);

        return {
          status: 'completed',
          executionId,
          finalResponse,
          iterations,
          toolExecutions,
          executionTime,
          metadata: {
            toolsAvailable: availableTools.length,
            skillsInvolved: [...new Set(toolExecutions.map(te => te.skillId))]
          }
        };

      } finally {
        clearTimeout(timeoutId);
      }

    } catch (error) {
      console.error('❌ Agent execution failed:', error);

      return {
        status: 'error',
        executionId,
        iterations,
        toolExecutions,
        executionTime: Date.now() - startTime,
        error: error instanceof Error ? error.message : String(error)
      };
    } finally {
      this.activeExecutions.delete(executionId);
    }
  }

  /**
   * Cancel an active agent execution
   */
  cancelExecution(executionId: string): boolean {
    const controller = this.activeExecutions.get(executionId);
    if (controller) {
      controller.abort();
      this.activeExecutions.delete(executionId);
      console.log(`🛑 Cancelled agent execution: ${executionId}`);
      return true;
    }
    return false;
  }

  /**
   * Get list of active execution IDs
   */
  getActiveExecutions(): string[] {
    return Array.from(this.activeExecutions.keys());
  }

  /**
   * Get execution status (placeholder - would need Redux integration)
   */
  getExecutionStatus(executionId: string): null {
    // This would typically integrate with Redux state
    // For now, just return null
    return null;
  }

  // =============================================================================
  // Private Helper Methods
  // =============================================================================

  /**
   * Execute a single tool call
   */
  private async executeSingleTool(
    toolCall: any,
    availableTools: any[],
    requireApproval: boolean,
    signal: AbortSignal
  ): Promise<AgentToolExecution> {
    const startTime = Date.now();

    try {
      // Find the tool and its associated skill
      const toolSchema = availableTools.find(t => t.function.name === toolCall.function.name);
      if (!toolSchema) {
        return {
          id: toolCall.id,
          toolName: toolCall.function.name,
          skillId: 'unknown',
          arguments: toolCall.function.arguments,
          status: 'error',
          startTime,
          endTime: Date.now(),
          errorMessage: `Tool not found: ${toolCall.function.name}`
        };
      }

      const skillId = (toolSchema.function as any).skillId;

      console.log(`🔧 Executing tool: ${skillId}.${toolCall.function.name}`);

      // TODO: Implement approval workflow if requireApproval is true

      // Execute the tool using the existing skill system
      const result = await this.toolRegistry.executeTool(
        skillId,
        toolCall.function.name,
        toolCall.function.arguments
      );

      console.log(`✅ Tool execution ${result.status}: ${toolCall.function.name}`);

      return {
        ...result,
        id: toolCall.id // Use the tool call ID from the AI
      };

    } catch (error) {
      const endTime = Date.now();
      console.error(`❌ Tool execution error: ${toolCall.function.name}`, error);

      return {
        id: toolCall.id,
        toolName: toolCall.function.name,
        skillId: 'unknown',
        arguments: toolCall.function.arguments,
        status: 'error',
        startTime,
        endTime,
        executionTimeMs: endTime - startTime,
        errorMessage: error instanceof Error ? error.message : String(error)
      };
    }
  }

  /**
   * Filter tools based on allowed skills and blocked tools
   */
  private filterTools(
    toolSchemas: any[],
    allowedSkills?: string[],
    blockedTools: string[] = []
  ): any[] {
    return toolSchemas.filter(tool => {
      const skillId = (tool.function as any).skillId;
      const toolName = tool.function.name;

      // Check if tool is blocked
      if (blockedTools.includes(toolName)) {
        return false;
      }

      // Check if skill is allowed (if allowedSkills is specified)
      if (allowedSkills && allowedSkills.length > 0) {
        return allowedSkills.includes(skillId);
      }

      return true;
    });
  }

  /**
   * Convert agent tool schema to OpenAI tool format
   */
  private convertToOpenAITool(toolSchema: any): OpenAITool {
    return {
      type: 'function',
      function: {
        name: toolSchema.function.name,
        description: toolSchema.function.description,
        parameters: toolSchema.function.parameters
      }
    };
  }
}