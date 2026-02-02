import { invoke } from '@tauri-apps/api/core';

import type { ConstitutionConfig } from '../constitution/types';
import type { MemoryManager } from '../memory/manager';
import type { LLMProvider, Message } from '../providers/interface';
import { compactSession, shouldCompact } from './compaction';
import { captureSessionEnd, shouldCaptureSession } from './session-capture';
import {
  appendCompactionMarker,
  appendMessage,
  appendSessionEndMarker,
  readMessages,
  writeSessionHeader,
} from './transcript';
import {
  DEFAULT_SESSION_CONFIG,
  type SessionConfig,
  type SessionEntry,
  type TranscriptMessage,
} from './types';

/**
 * SessionManager handles session lifecycle:
 * - Creating new sessions
 * - Loading existing sessions
 * - Appending messages
 * - Triggering compaction when needed
 * - Updating the session index
 */
export class SessionManager {
  private config: SessionConfig;
  private currentSessionId: string | null = null;
  private currentEntry: SessionEntry | null = null;
  private messageBuffer: Message[] = [];

  constructor(config: Partial<SessionConfig> = {}) {
    this.config = { ...DEFAULT_SESSION_CONFIG, ...config };
  }

  /** Initialize the sessions directory */
  async init(): Promise<void> {
    await invoke('ai_sessions_init');
  }

  /** Get the current session ID */
  getSessionId(): string | null {
    return this.currentSessionId;
  }

  /** Get the current session entry */
  getEntry(): SessionEntry | null {
    return this.currentEntry;
  }

  /** Get buffered messages for the current session */
  getMessages(): Message[] {
    return [...this.messageBuffer];
  }

  /**
   * Create a new session.
   */
  async createSession(params: {
    model: string;
    label?: string;
    channel?: string;
  }): Promise<string> {
    const sessionId = crypto.randomUUID();
    const now = Date.now();

    const entry: SessionEntry = {
      sessionId,
      updatedAt: now,
      sessionFile: `${sessionId}.jsonl`,
      inputTokens: 0,
      outputTokens: 0,
      totalTokens: 0,
      model: params.model,
      compactionCount: 0,
      label: params.label,
      channel: params.channel,
    };

    // Write session header to transcript
    await writeSessionHeader(sessionId);

    // Update index
    await invoke('ai_sessions_update_index', { sessionId, entry });

    this.currentSessionId = sessionId;
    this.currentEntry = entry;
    this.messageBuffer = [];

    return sessionId;
  }

  /**
   * Load an existing session by ID.
   */
  async loadSession(sessionId: string): Promise<void> {
    // Load session entry from index
    const index = await invoke<Record<string, SessionEntry>>('ai_sessions_load_index');
    const entry = index[sessionId];
    if (!entry) {
      throw new Error(`Session not found: ${sessionId}`);
    }

    // Load messages from transcript
    const transcriptMessages = await readMessages(sessionId);

    // Convert transcript messages to Message format
    this.messageBuffer = transcriptMessages.map(tm => ({
      role: tm.message.role,
      content: tm.message.content.map(c => {
        if (c.type === 'text') {
          return { type: 'text' as const, text: c.text || '' };
        }
        return c as Message['content'][0];
      }),
      usage: tm.message.usage
        ? {
            inputTokens: tm.message.usage.inputTokens,
            outputTokens: tm.message.usage.outputTokens,
            totalTokens: tm.message.usage.inputTokens + tm.message.usage.outputTokens,
          }
        : undefined,
    }));

    this.currentSessionId = sessionId;
    this.currentEntry = entry;
  }

  /**
   * Add a user message to the session.
   */
  async addUserMessage(text: string): Promise<void> {
    if (!this.currentSessionId) {
      throw new Error('No active session');
    }

    const message: Message = { role: 'user', content: [{ type: 'text', text }] };

    this.messageBuffer.push(message);

    await appendMessage(this.currentSessionId, { role: 'user', content: [{ type: 'text', text }] });
  }

  /**
   * Add an assistant response to the session.
   */
  async addAssistantMessage(message: Message): Promise<void> {
    if (!this.currentSessionId || !this.currentEntry) {
      throw new Error('No active session');
    }

    this.messageBuffer.push(message);

    await appendMessage(this.currentSessionId, {
      role: 'assistant',
      content: message.content.map(c => {
        if (c.type === 'text') return { type: 'text', text: c.text };
        return c as TranscriptMessage['message']['content'][0];
      }),
      usage: message.usage
        ? { inputTokens: message.usage.inputTokens, outputTokens: message.usage.outputTokens }
        : undefined,
    });

    // Update token counts
    if (message.usage) {
      this.currentEntry.inputTokens += message.usage.inputTokens;
      this.currentEntry.outputTokens += message.usage.outputTokens;
      this.currentEntry.totalTokens += message.usage.totalTokens;
    }
    this.currentEntry.updatedAt = Date.now();

    await invoke('ai_sessions_update_index', {
      sessionId: this.currentSessionId,
      entry: this.currentEntry,
    });
  }

  /**
   * Check if compaction is needed and execute it.
   */
  async maybeCompact(params: {
    provider: LLMProvider;
    constitution: ConstitutionConfig;
    memoryManager: MemoryManager;
  }): Promise<boolean> {
    if (!this.currentSessionId || !this.currentEntry) return false;

    if (!shouldCompact(this.messageBuffer, this.config)) {
      return false;
    }

    const result = await compactSession({
      provider: params.provider,
      constitution: params.constitution,
      memoryManager: params.memoryManager,
      messages: this.messageBuffer,
      compactionCount: this.currentEntry.compactionCount,
      lastFlushCompactionCount: this.currentEntry.memoryFlushCompactionCount,
      config: this.config,
    });

    // Update buffer with compacted messages
    this.messageBuffer = result.compactedMessages;

    // Write compaction marker to transcript
    await appendCompactionMarker(
      this.currentSessionId,
      result.compactionCount,
      result.summary,
      result.compactedMessages.length
    );

    // Update entry
    this.currentEntry.compactionCount = result.compactionCount;
    this.currentEntry.memoryFlushCompactionCount = result.memoryFlushCompactionCount;
    this.currentEntry.memoryFlushAt = Date.now();
    this.currentEntry.updatedAt = Date.now();

    await invoke('ai_sessions_update_index', {
      sessionId: this.currentSessionId,
      entry: this.currentEntry,
    });

    return true;
  }

  /**
   * End the current session, optionally capturing durable facts.
   *
   * If the session has enough substance (>= 2 user turns, >= 100 tokens),
   * runs a lightweight memory flush before closing.
   */
  async endSession(params: {
    provider: LLMProvider;
    constitution: ConstitutionConfig;
    memoryManager: MemoryManager;
  }): Promise<{ memoryCaptured: boolean }> {
    if (!this.currentSessionId || !this.currentEntry) {
      return { memoryCaptured: false };
    }

    let memoryCaptured = false;

    if (shouldCaptureSession(this.messageBuffer, this.currentEntry)) {
      try {
        const result = await captureSessionEnd({
          provider: params.provider,
          constitution: params.constitution,
          memoryManager: params.memoryManager,
          messages: this.messageBuffer,
          sessionId: this.currentSessionId,
          sessionEntry: this.currentEntry,
          toolCaptureConfig: this.config.toolCaptureConfig,
        });
        memoryCaptured = result.captured;

        // Update entry with flush tracking
        if (memoryCaptured) {
          this.currentEntry.memoryFlushAt = Date.now();
          this.currentEntry.memoryFlushCompactionCount = this.currentEntry.compactionCount + 1;
          this.currentEntry.updatedAt = Date.now();

          await invoke('ai_sessions_update_index', {
            sessionId: this.currentSessionId,
            entry: this.currentEntry,
          });
        }
      } catch {
        // Session-end capture failed — non-fatal
      }
    }

    // Write session-end marker
    await appendSessionEndMarker(this.currentSessionId, memoryCaptured);

    // Clear current session state
    this.currentSessionId = null;
    this.currentEntry = null;
    this.messageBuffer = [];

    return { memoryCaptured };
  }

  /**
   * List all sessions.
   */
  async listSessions(): Promise<SessionEntry[]> {
    const index = await invoke<Record<string, SessionEntry>>('ai_sessions_load_index');
    return Object.values(index).sort((a, b) => b.updatedAt - a.updatedAt);
  }

  /**
   * Delete a session.
   */
  async deleteSession(sessionId: string): Promise<void> {
    await invoke('ai_sessions_delete', { sessionId });
    if (this.currentSessionId === sessionId) {
      this.currentSessionId = null;
      this.currentEntry = null;
      this.messageBuffer = [];
    }
  }
}
