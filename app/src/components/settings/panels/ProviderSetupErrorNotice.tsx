import { useT } from '../../../lib/i18n/I18nContext';

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

function fillTemplate(template: string, replacements: Record<string, string>): string {
  return Object.entries(replacements).reduce(
    (result, [key, value]) => result.replaceAll(`{${key}}`, value),
    template
  );
}

export function presentProviderSetupError(
  raw: string,
  t: (key: string, fallback?: string) => string
): ProviderErrorPresentation {
  const details = raw.trim() || t('providerSetup.error.defaultDetails', 'Provider setup failed.');
  const couldNotReach = details.match(/^Could not reach\s+([^:]+):\s*(.*)$/i);
  const provider = couldNotReach?.[1]?.trim();
  const cause = couldNotReach?.[2]?.trim() || details;
  const status = cause.match(/provider returned\s+(\d{3})/i)?.[1];
  const providerLabel = provider || t('providerSetup.error.providerFallback', 'The provider');

  let summary: string | null = null;

  if (status === '401' || status === '403') {
    summary = fillTemplate(
      t(
        'providerSetup.error.credentialsRejected',
        '{provider} rejected the credentials. Check the API key and try again.'
      ),
      { provider: providerLabel }
    );
  } else if (status === '404') {
    summary = fillTemplate(
      t(
        'providerSetup.error.endpointNotRecognized',
        '{provider} did not recognize the endpoint. Check the base URL and try again.'
      ),
      { provider: providerLabel }
    );
  } else if (status && Number(status) >= 500) {
    summary = fillTemplate(
      t(
        'providerSetup.error.providerUnavailable',
        '{provider} is unavailable right now. Try again or check the provider status.'
      ),
      { provider: providerLabel }
    );
  } else if (/HTTP request failed|error sending request|timed out|ECONNREFUSED/i.test(cause)) {
    summary = fillTemplate(
      t(
        'providerSetup.error.unreachable',
        'Could not reach {provider}. Check the endpoint URL and network connection, then try again.'
      ),
      { provider: providerLabel }
    );
  }

  if (!summary) {
    const jsonMessage = findProviderJsonMessage(cause);
    if (jsonMessage) {
      summary = provider
        ? fillTemplate(
            t(
              'providerSetup.error.couldNotReachWithMessage',
              'Could not reach {provider}: {message}'
            ),
            { provider, message: cleanProviderMessage(jsonMessage) }
          )
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
  const { t } = useT();
  const { summary, details } = presentProviderSetupError(error, t);
  const hasDetails = details !== summary;

  return (
    <div
      role="alert"
      className="max-w-full min-w-0 overflow-hidden rounded-md border border-red-200 dark:border-red-500/30 bg-red-50 dark:bg-red-500/10 px-3 py-2 text-xs text-red-700 dark:text-red-300">
      <p className="break-words font-medium leading-relaxed [overflow-wrap:anywhere]">{summary}</p>
      {hasDetails ? (
        <details className="mt-2 max-w-full min-w-0">
          <summary className="cursor-pointer text-[11px] font-medium text-red-700 dark:text-red-200">
            {t('providerSetup.error.technicalDetails')}
          </summary>
          <pre className="mt-1 max-h-32 max-w-full overflow-auto whitespace-pre-wrap break-words rounded border border-red-200/70 dark:border-red-500/30 bg-white/70 dark:bg-neutral-950/40 p-2 font-mono text-[11px] leading-relaxed text-red-800 dark:text-red-200 [overflow-wrap:anywhere]">
            {details}
          </pre>
        </details>
      ) : null}
    </div>
  );
};
