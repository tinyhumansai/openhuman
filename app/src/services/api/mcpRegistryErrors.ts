export type McpRegistryErrorKind = 'not_found' | 'network' | 'unavailable';

const FALLBACK_MESSAGES: Record<McpRegistryErrorKind, string> = {
  not_found:
    'Server not found in registry. Check the server name and try again, browse available MCP servers, or add the server manually by URL.',
  network:
    'Could not reach the MCP registry. Check your connection and try again, or add the server manually by URL.',
  unavailable:
    'The MCP registry is unavailable right now. Try again later, browse available MCP servers, or add the server manually by URL.',
};

export class McpRegistryUserError extends Error {
  readonly kind: McpRegistryErrorKind;
  readonly rawMessage: string;

  constructor(kind: McpRegistryErrorKind, rawMessage: string) {
    super(FALLBACK_MESSAGES[kind]);
    this.name = 'McpRegistryUserError';
    this.kind = kind;
    this.rawMessage = rawMessage;
  }
}

function rawErrorMessage(error: unknown): string {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === 'string') return error;
  if (error && typeof error === 'object') {
    const maybeMessage = (error as { message?: unknown }).message;
    if (typeof maybeMessage === 'string' && maybeMessage.trim()) return maybeMessage;
  }
  return 'Unknown MCP registry error';
}

function statusCodes(message: string): number[] {
  const codes: number[] = [];
  for (const match of message.matchAll(/\bHTTP\s+(\d{3})\b/gi)) {
    codes.push(Number(match[1]));
  }
  for (const match of message.matchAll(/["']?status["']?\s*:\s*(\d{3})/gi)) {
    codes.push(Number(match[1]));
  }
  return codes;
}

export function classifyMcpRegistryError(error: unknown): McpRegistryErrorKind {
  if (error instanceof McpRegistryUserError) return error.kind;

  const message = rawErrorMessage(error);
  const codes = statusCodes(message);

  if (codes.includes(404) || /server not found/i.test(message)) return 'not_found';
  if (codes.some(code => code === 429 || code >= 500)) return 'unavailable';
  if (
    /timed?\s*out|timeout|abort|failed to fetch|networkerror|econnrefused|enotfound|error sending request|client error \(connect\)|request failed|read failed|failed to respond|all MCP registries failed/i.test(
      message
    )
  ) {
    return 'network';
  }
  if (/returned HTTP|registry .*failed|HTTP\s+\d{3}/i.test(message)) return 'unavailable';

  return 'unavailable';
}

export function normalizeMcpRegistryError(error: unknown): McpRegistryUserError {
  if (error instanceof McpRegistryUserError) return error;
  return new McpRegistryUserError(classifyMcpRegistryError(error), rawErrorMessage(error));
}

export function getMcpRegistryErrorKind(error: unknown): McpRegistryErrorKind | null {
  if (!error) return null;
  return classifyMcpRegistryError(error);
}
