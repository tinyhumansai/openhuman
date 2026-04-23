import type { ToolTimelineEntry } from '../store/chatRuntimeSlice';

interface ParsedToolArgs {
  agent_id?: string;
  prompt?: string;
  toolkit?: string;
}

export function formatTimelineEntry(entry: ToolTimelineEntry): { title: string; detail?: string } {
  const parsedArgs = parseToolArgs(entry.argsBuffer);

  if (entry.name === 'spawn_subagent' && parsedArgs?.agent_id === 'integrations_agent') {
    const provider =
      inferIntegrationName(parsedArgs.toolkit) ?? inferIntegrationNameFromPrompt(parsedArgs.prompt);
    return {
      title: provider ? `Checking your ${provider}` : 'Checking your connected app',
      detail: parsedArgs.prompt?.trim() || entry.detail,
    };
  }

  if (entry.name === 'integrations_agent' || entry.name === 'subagent:integrations_agent') {
    const provider =
      inferIntegrationName(entry.sourceToolName) ??
      inferIntegrationName(parsedArgs?.toolkit) ??
      inferIntegrationNameFromPrompt(entry.detail) ??
      inferIntegrationNameFromPrompt(parsedArgs?.prompt);

    return {
      title: provider ? `Checking your ${provider}` : 'Checking your connected app',
      detail: entry.detail,
    };
  }

  if (entry.name === 'subagent:researcher' || entry.name === 'researcher') {
    return { title: 'Researching', detail: entry.detail };
  }
  if (entry.name === 'composio_list_connections') {
    return { title: 'Viewing your Connections', detail: entry.detail };
  }
  if (entry.name === 'subagent:orchestrator' || entry.name === 'orchestrator') {
    return { title: 'Planning next steps', detail: entry.detail };
  }
  if (entry.name === 'subagent:critic' || entry.name === 'critic') {
    return { title: 'Reviewing the work', detail: entry.detail };
  }
  if (entry.name === 'subagent:tools_agent' || entry.name === 'tools_agent') {
    return { title: 'Using tools', detail: entry.detail };
  }
  if (entry.name === 'subagent:code_executor' || entry.name === 'code_executor') {
    return { title: 'Running code', detail: entry.detail };
  }

  if (entry.name.startsWith('delegate_')) {
    const provider = inferIntegrationName(entry.name);
    return {
      title: provider ? `Checking your ${provider}` : humanizeIdentifier(entry.name),
      detail: entry.detail ?? parsedArgs?.prompt,
    };
  }

  return {
    title: entry.displayName ?? humanizeIdentifier(entry.name),
    detail: entry.detail ?? parsedArgs?.prompt,
  };
}

export function promptFromArgsBuffer(argsBuffer?: string): string | undefined {
  return parseToolArgs(argsBuffer)?.prompt?.trim() || undefined;
}

export function inferIntegrationName(input?: string): string | undefined {
  if (!input) return undefined;

  const delegateMatch = input.match(/^delegate_(.+)$/);
  if (delegateMatch) {
    return humanizeIdentifier(delegateMatch[1]);
  }

  const toolkitMatch = input.match(
    /^(gmail|notion|github|slack|discord|linear|jira|google_calendar|google_drive|calendar)$/i
  );
  if (toolkitMatch) {
    return humanizeIdentifier(toolkitMatch[1]);
  }

  return undefined;
}

function inferIntegrationNameFromPrompt(prompt?: string): string | undefined {
  if (!prompt) return undefined;
  const known = [
    'Notion',
    'Gmail',
    'GitHub',
    'Slack',
    'Discord',
    'Linear',
    'Jira',
    'Google Calendar',
    'Google Drive',
  ];

  const lower = prompt.toLowerCase();
  return known.find(name => lower.includes(name.toLowerCase()));
}

function parseToolArgs(argsBuffer?: string): ParsedToolArgs | null {
  if (!argsBuffer) return null;
  try {
    const parsed = JSON.parse(argsBuffer) as ParsedToolArgs;
    return parsed && typeof parsed === 'object' ? parsed : null;
  } catch {
    return null;
  }
}

function humanizeIdentifier(value: string): string {
  return value
    .replace(/^subagent:/, '')
    .replace(/^delegate_/, '')
    .replace(/_/g, ' ')
    .replace(/\b\w/g, char => char.toUpperCase());
}
