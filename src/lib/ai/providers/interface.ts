/** A single message in a conversation */
export interface Message {
  role: 'system' | 'user' | 'assistant' | 'tool';
  content: MessageContent[];
  /** Tool call results */
  toolCallId?: string;
  /** Token usage for assistant messages */
  usage?: TokenUsage;
}

/** Content block types */
export type MessageContent =
  | { type: 'text'; text: string }
  | { type: 'tool_use'; id: string; name: string; input: Record<string, unknown> }
  | { type: 'tool_result'; toolUseId: string; content: string; isError?: boolean };

/** Token usage tracking */
export interface TokenUsage {
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
}

/** Tool definition for function calling */
export interface ToolDefinition {
  name: string;
  description: string;
  parameters: Record<string, unknown>; // JSON Schema
}

/** Streaming chunk from an LLM */
export interface StreamChunk {
  type: 'text' | 'tool_use_start' | 'tool_use_delta' | 'tool_use_end' | 'done';
  text?: string;
  toolUse?: {
    id: string;
    name: string;
    input: string; // Partial JSON
  };
  usage?: TokenUsage;
}

/** Chat completion parameters */
export interface ChatParams {
  systemPrompt: string;
  messages: Message[];
  tools?: ToolDefinition[];
  maxTokens?: number;
  temperature?: number;
  stopSequences?: string[];
}

/**
 * Abstract LLM provider interface.
 * Implementations handle the specifics of each provider's API.
 */
export interface LLMProvider {
  /** Unique provider identifier */
  id: string;
  /** Human-readable provider name */
  name: string;

  /**
   * Stream a chat completion.
   * Yields chunks as they arrive from the provider.
   */
  chat(params: ChatParams): AsyncIterable<StreamChunk>;

  /**
   * Non-streaming chat completion.
   * Returns the complete response.
   */
  complete(params: ChatParams): Promise<Message>;

  /** Check if the provider is configured and ready */
  isAvailable(): boolean;
}

/** Configuration for an LLM provider */
export interface LLMProviderConfig {
  /** Provider identifier */
  id: string;
  /** API endpoint URL */
  endpoint?: string;
  /** API key (stored securely) */
  apiKey?: string;
  /** Default model to use */
  model?: string;
  /** Default max tokens */
  maxTokens?: number;
  /** Default temperature */
  temperature?: number;
}
