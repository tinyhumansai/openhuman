export type ProviderErrorPresentation = { summary: string; details: string };

function decodeJsonString(value: string): string {
  try {
    return JSON.parse(`"${value}"`) as string;
  } catch {
    return value;
  }
}

function findProviderJsonMessage(raw: string): string | null {
  const match = raw.match(/"message"\s*:\s*"((?:\\.|[^"\\])*)"/);
  return match ? decodeJsonString(match[1]) : null;
}

function cleanProviderMessage(message: string): string {
  return message.replace(/\s+/g, ' ').trim();
}

export function presentProviderSetupError(raw: string): ProviderErrorPresentation {
  const details = raw.trim() || 'Provider setup failed.';
  const couldNotReach = details.match(/^Could not reach\s+([^:]+):\s*(.*)$/i);
  const provider = couldNotReach?.[1]?.trim();
  const cause = couldNotReach?.[2]?.trim() || details;
  const status = cause.match(/provider returned\s+(\d{3})/i)?.[1];
  const providerLabel = provider || 'The provider';

  let summary: string | null = null;

  if (status === '401' || status === '403') {
    summary = `${providerLabel} rejected the credentials. Check the API key and try again.`;
  } else if (status === '404') {
    summary = `${providerLabel} did not recognize the endpoint. Check the base URL and try again.`;
  } else if (status && Number(status) >= 500) {
    summary = `${providerLabel} is unavailable right now. Try again or check the provider status.`;
  } else if (/HTTP request failed|error sending request|timed out|ECONNREFUSED/i.test(cause)) {
    summary = `Could not reach ${providerLabel}. Check the endpoint URL and network connection, then try again.`;
  }

  if (!summary) {
    const jsonMessage = findProviderJsonMessage(cause);
    if (jsonMessage) {
      summary = provider
        ? `Could not reach ${provider}: ${cleanProviderMessage(jsonMessage)}`
        : cleanProviderMessage(jsonMessage);
    }
  }

  if (!summary) {
    summary = cleanProviderMessage(cause);
  }

  if (summary.length > 220) {
    summary = `${summary.slice(0, 217).trimEnd()}...`;
  }

  return { summary, details };
}

export const ProviderSetupErrorNotice = ({ error }: { error: string }) => {
  const { summary, details } = presentProviderSetupError(error);
  const hasDetails = details !== summary;

  return (
    <div
      role="alert"
      className="max-w-full min-w-0 overflow-hidden rounded-md border border-red-200 dark:border-red-500/30 bg-red-50 dark:bg-red-500/10 px-3 py-2 text-xs text-red-700 dark:text-red-300">
      <p className="break-words font-medium leading-relaxed [overflow-wrap:anywhere]">{summary}</p>
      {hasDetails ? (
        <details className="mt-2 max-w-full min-w-0">
          <summary className="cursor-pointer text-[11px] font-medium text-red-700 dark:text-red-200">
            Technical details
          </summary>
          <pre className="mt-1 max-h-32 max-w-full overflow-auto whitespace-pre-wrap break-words rounded border border-red-200/70 dark:border-red-500/30 bg-white/70 dark:bg-neutral-950/40 p-2 font-mono text-[11px] leading-relaxed text-red-800 dark:text-red-200 [overflow-wrap:anywhere]">
            {details}
          </pre>
        </details>
      ) : null}
    </div>
  );
};
