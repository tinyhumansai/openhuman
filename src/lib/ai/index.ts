/**
 * AlphaHuman AI Intelligence System
 *
 * Client-side AI system inspired by OpenClaw's architecture, adapted for Tauri.
 *
 * Modules:
 * - **constitution/** — Agent safety & compliance framework
 * - **memory/** — JSON file-based index + vector search memory storage
 * - **entities/** — Remote Neo4j entity graph via backend API
 * - **prompts/** — Modular system prompt construction
 * - **sessions/** — JSONL session transcripts with compaction
 * - **providers/** — Pluggable LLM and embedding providers
 * - **tools/** — AI tool definitions (memory_search, memory_write, etc.)
 *
 * Skills are .md files or Python scripts in the skills/ submodule.
 */

// Constitution
export { loadConstitution, parseConstitution } from './constitution/loader';
export { validateMemoryContent, validateAction, sanitizeForMemory } from './constitution/validator';
export type {
  ConstitutionConfig,
  ConstitutionValidation,
  ConstitutionViolation,
} from './constitution/types';

// Memory
export { MemoryManager } from './memory/manager';
export { chunkMarkdown, sha256 } from './memory/chunker';
export { hybridSearch } from './memory/hybrid-search';
export { MemoryEncryption } from './memory/encryption';
export type { FileRecord, ChunkRecord, SearchResult, MemoryConfig } from './memory/types';
export { DEFAULT_MEMORY_CONFIG, MEMORY_PATHS } from './memory/types';

// Entities
export { EntityManager } from './entities/manager';
export { EntityQuery } from './entities/query';
export type {
  Entity,
  EntityRelation,
  EntityTag,
  EntitySearchResult,
  EntityType,
  EntitySource,
  RelationType,
} from './entities/types';

// Prompts
export { buildSystemPrompt } from './prompts/system-prompt';
export type { SystemPromptParams } from './prompts/system-prompt';
export type { AgentIdentity } from './prompts/sections/identity';
export type { CryptoIntelligenceContext } from './prompts/sections/crypto-intelligence';
export type { UserContext } from './prompts/sections/context';
export {
  MEMORY_FLUSH_TEMPLATE,
  COMPACTION_SUMMARY_TEMPLATE,
  SILENT_TOKEN,
} from './prompts/templates';

// Sessions
export { SessionManager } from './sessions/manager';
export type { SessionEntry, SessionConfig, SessionState } from './sessions/types';
export { DEFAULT_SESSION_CONFIG } from './sessions/types';

// Providers
export { CustomLLMProvider } from './providers/custom';
export { OpenAIEmbeddingProvider } from './providers/openai';
export { NullEmbeddingProvider } from './providers/embeddings';
export type {
  LLMProvider,
  LLMProviderConfig,
  ChatParams,
  StreamChunk,
  Message,
  MessageContent,
  TokenUsage,
  ToolDefinition,
} from './providers/interface';
export type { EmbeddingProvider, EmbeddingProviderConfig } from './providers/embeddings';

// Tools
export { ToolRegistry } from './tools/registry';
export { createMemorySearchTool } from './tools/memory-search';
export { createMemoryReadTool } from './tools/memory-read';
export { createMemoryWriteTool } from './tools/memory-write';
export { createWebSearchTool } from './tools/web-search';
export type { AITool, ToolResult } from './tools/registry';
