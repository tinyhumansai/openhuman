import type { EmbeddingProvider, EmbeddingProviderConfig } from './embeddings';

/**
 * OpenAI embedding provider.
 * Used as a fallback for high-quality embeddings when no local model is available.
 */
export class OpenAIEmbeddingProvider implements EmbeddingProvider {
  id = 'openai';
  model: string;
  dimensions: number;
  private apiKey: string;
  private endpoint: string;

  constructor(config: EmbeddingProviderConfig) {
    this.apiKey = config.apiKey || '';
    this.endpoint = config.endpoint || 'https://api.openai.com';
    this.model = config.model || 'text-embedding-3-small';
    this.dimensions = this.model.includes('3-large') ? 3072 : 1536;
  }

  async embedQuery(text: string): Promise<number[]> {
    const [result] = await this.embedBatch([text]);
    return result;
  }

  async embedBatch(texts: string[]): Promise<number[][]> {
    if (!this.apiKey) {
      throw new Error('OpenAI API key not configured');
    }

    const response = await fetch(this.endpoint + '/v1/embeddings', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${this.apiKey}` },
      body: JSON.stringify({ model: this.model, input: texts }),
    });

    if (!response.ok) {
      throw new Error(`OpenAI embeddings error: ${response.status} ${response.statusText}`);
    }

    const data = await response.json();
    return data.data
      .sort((a: { index: number }, b: { index: number }) => a.index - b.index)
      .map((item: { embedding: number[] }) => item.embedding);
  }
}
