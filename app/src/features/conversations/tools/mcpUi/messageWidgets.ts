import { readOpenHumanToolArtifact } from '../../../../providers/assistantUiMessages';
import { hasWidget, type McpUiPresentation, readMcpUiPresentation } from './types';

export interface MessageWidget {
  /** Stable per widget template: the same key keeps the same frame mounted. */
  key: string;
  presentation: McpUiPresentation;
  /** Index of the message part the latest presentation came from. */
  partIndex: number;
}

interface ToolPartLike {
  type: string;
  artifact?: unknown;
}

/** One key per widget template: its server and its resource or inline document. */
export function widgetKey(presentation: McpUiPresentation): string {
  const source = presentation.inline_id
    ? `inline:${presentation.inline_id}`
    : `uri:${presentation.resource_uri ?? ''}`;
  return `${presentation.server_id ?? ''}|${source}`;
}

/**
 * The widgets one assistant message offers: the latest presentation of each
 * template, ordered by where that latest call sits in the message.
 */
export function collectMessageWidgets(parts: readonly ToolPartLike[]): MessageWidget[] {
  const latest = new Map<string, MessageWidget>();
  parts.forEach((part, partIndex) => {
    if (part.type !== 'tool-call') return;
    const presentation = readMcpUiPresentation(
      readOpenHumanToolArtifact(part.artifact)?.structured
    );
    if (!presentation || !hasWidget(presentation)) return;
    const key = widgetKey(presentation);
    latest.delete(key);
    latest.set(key, { key, presentation, partIndex });
  });
  return [...latest.values()].sort((a, b) => a.partIndex - b.partIndex);
}
