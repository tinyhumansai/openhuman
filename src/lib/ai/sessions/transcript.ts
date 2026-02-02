import { invoke } from '@tauri-apps/api/core';

import type {
  CompactionMarker,
  SessionEndMarker,
  SessionHeader,
  TranscriptLine,
  TranscriptMessage,
} from './types';

/**
 * Append-only JSONL transcript operations.
 * Each session has one JSONL file, one JSON object per line.
 */

/**
 * Write the session header (first line of a new transcript).
 */
export async function writeSessionHeader(sessionId: string): Promise<void> {
  const header: SessionHeader = {
    type: 'session',
    version: '1.0',
    sessionId,
    timestamp: new Date().toISOString(),
  };

  await invoke('ai_sessions_append_transcript', { sessionId, line: JSON.stringify(header) });
}

/**
 * Append a message to the transcript.
 */
export async function appendMessage(
  sessionId: string,
  message: TranscriptMessage['message']
): Promise<void> {
  const entry: TranscriptMessage = {
    type: 'message',
    timestamp: new Date().toISOString(),
    message,
  };

  await invoke('ai_sessions_append_transcript', { sessionId, line: JSON.stringify(entry) });
}

/**
 * Append a compaction marker to the transcript.
 */
export async function appendCompactionMarker(
  sessionId: string,
  compactionCount: number,
  summary: string,
  preservedMessages: number
): Promise<void> {
  const marker: CompactionMarker = {
    type: 'compaction',
    timestamp: new Date().toISOString(),
    compactionCount,
    summary,
    preservedMessages,
  };

  await invoke('ai_sessions_append_transcript', { sessionId, line: JSON.stringify(marker) });
}

/**
 * Read and parse all lines from a session transcript.
 */
export async function readTranscript(sessionId: string): Promise<TranscriptLine[]> {
  const lines = await invoke<string[]>('ai_sessions_read_transcript', { sessionId });

  return lines
    .map(line => {
      try {
        return JSON.parse(line) as TranscriptLine;
      } catch {
        return null;
      }
    })
    .filter((l): l is TranscriptLine => l !== null);
}

/**
 * Extract all messages from a transcript (excluding headers and markers).
 */
export async function readMessages(sessionId: string): Promise<TranscriptMessage[]> {
  const lines = await readTranscript(sessionId);
  return lines.filter((l): l is TranscriptMessage => l.type === 'message');
}

/**
 * Append a session end marker to the transcript.
 */
export async function appendSessionEndMarker(
  sessionId: string,
  memoryCaptured: boolean
): Promise<void> {
  const marker: SessionEndMarker = {
    type: 'session_end',
    timestamp: new Date().toISOString(),
    memoryCaptured,
  };

  await invoke('ai_sessions_append_transcript', { sessionId, line: JSON.stringify(marker) });
}

/**
 * Get the latest compaction marker from a transcript.
 */
export async function getLastCompaction(sessionId: string): Promise<CompactionMarker | null> {
  const lines = await readTranscript(sessionId);
  const compactions = lines.filter((l): l is CompactionMarker => l.type === 'compaction');
  return compactions.length > 0 ? compactions[compactions.length - 1] : null;
}
