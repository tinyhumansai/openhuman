import type {
  ChatParams,
  LLMProvider,
  LLMProviderConfig,
  Message,
  StreamChunk,
  TokenUsage,
} from './interface';

/**
 * Custom LLM provider for connecting to your own model endpoint.
 *
 * Expects an OpenAI-compatible API (POST /v1/chat/completions).
 * This is the primary provider — designed for your custom model.
 */
export class CustomLLMProvider implements LLMProvider {
  id: string;
  name: string;
  private config: LLMProviderConfig;

  constructor(config: LLMProviderConfig) {
    this.id = config.id || 'custom';
    this.name = 'Custom LLM';
    this.config = config;
  }

  isAvailable(): boolean {
    return Boolean(this.config.endpoint);
  }

  async *chat(params: ChatParams): AsyncIterable<StreamChunk> {
    if (!this.config.endpoint) {
      throw new Error('Custom LLM endpoint not configured');
    }

    const body = this.buildRequestBody(params, true);

    const response = await fetch(this.config.endpoint + '/v1/chat/completions', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        ...(this.config.apiKey ? { Authorization: `Bearer ${this.config.apiKey}` } : {}),
      },
      body: JSON.stringify(body),
    });

    if (!response.ok) {
      throw new Error(`Custom LLM error: ${response.status} ${response.statusText}`);
    }

    const reader = response.body?.getReader();
    if (!reader) {
      throw new Error('No response body');
    }

    const decoder = new TextDecoder();
    let buffer = '';

    while (true) {
      const { done, value } = await reader.read();
      if (done) break;

      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split('\n');
      buffer = lines.pop() || '';

      for (const line of lines) {
        const trimmed = line.trim();
        if (!trimmed || !trimmed.startsWith('data: ')) continue;

        const data = trimmed.slice(6);
        if (data === '[DONE]') {
          yield { type: 'done' };
          return;
        }

        try {
          const parsed = JSON.parse(data);
          const choice = parsed.choices?.[0];
          if (!choice) continue;

          const delta = choice.delta;
          if (delta?.content) {
            yield { type: 'text', text: delta.content };
          }

          // Handle tool calls in streaming
          if (delta?.tool_calls) {
            for (const tc of delta.tool_calls) {
              if (tc.function?.name) {
                yield {
                  type: 'tool_use_start',
                  toolUse: {
                    id: tc.id || '',
                    name: tc.function.name,
                    input: tc.function.arguments || '',
                  },
                };
              } else if (tc.function?.arguments) {
                yield {
                  type: 'tool_use_delta',
                  toolUse: { id: tc.id || '', name: '', input: tc.function.arguments },
                };
              }
            }
          }

          // Usage info at the end
          if (parsed.usage) {
            yield {
              type: 'done',
              usage: {
                inputTokens: parsed.usage.prompt_tokens || 0,
                outputTokens: parsed.usage.completion_tokens || 0,
                totalTokens: parsed.usage.total_tokens || 0,
              },
            };
          }
        } catch {
          // Skip unparseable lines
        }
      }
    }

    yield { type: 'done' };
  }

  async complete(params: ChatParams): Promise<Message> {
    if (!this.config.endpoint) {
      throw new Error('Custom LLM endpoint not configured');
    }

    const body = this.buildRequestBody(params, false);

    const response = await fetch(this.config.endpoint + '/v1/chat/completions', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        ...(this.config.apiKey ? { Authorization: `Bearer ${this.config.apiKey}` } : {}),
      },
      body: JSON.stringify(body),
    });

    if (!response.ok) {
      throw new Error(`Custom LLM error: ${response.status} ${response.statusText}`);
    }

    const data = await response.json();
    const choice = data.choices?.[0];

    const usage: TokenUsage = {
      inputTokens: data.usage?.prompt_tokens || 0,
      outputTokens: data.usage?.completion_tokens || 0,
      totalTokens: data.usage?.total_tokens || 0,
    };

    const content: Message['content'] = [];

    if (choice?.message?.content) {
      content.push({ type: 'text', text: choice.message.content });
    }

    if (choice?.message?.tool_calls) {
      for (const tc of choice.message.tool_calls) {
        content.push({
          type: 'tool_use',
          id: tc.id,
          name: tc.function.name,
          input: JSON.parse(tc.function.arguments || '{}'),
        });
      }
    }

    return { role: 'assistant', content, usage };
  }

  private buildRequestBody(params: ChatParams, stream: boolean) {
    const messages = [
      { role: 'system' as const, content: params.systemPrompt },
      ...params.messages.map(m => ({
        role: m.role,
        content:
          m.content.length === 1 && m.content[0].type === 'text' ? m.content[0].text : m.content,
        ...(m.toolCallId ? { tool_call_id: m.toolCallId } : {}),
      })),
    ];

    const body: Record<string, unknown> = {
      model: this.config.model || 'default',
      messages,
      stream,
      max_tokens: params.maxTokens ?? this.config.maxTokens ?? 4096,
    };

    if (params.temperature !== undefined) {
      body.temperature = params.temperature;
    } else if (this.config.temperature !== undefined) {
      body.temperature = this.config.temperature;
    }

    if (params.tools?.length) {
      body.tools = params.tools.map(t => ({
        type: 'function',
        function: { name: t.name, description: t.description, parameters: t.parameters },
      }));
    }

    if (params.stopSequences?.length) {
      body.stop = params.stopSequences;
    }

    return body;
  }
}
