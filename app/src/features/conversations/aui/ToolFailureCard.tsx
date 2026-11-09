import { ToolError } from '../../../components/assistant-ui/elements/tool-error';
import { useT } from '../../../lib/i18n/I18nContext';
import type { ToolFailureExplanation } from '../../../store/chatRuntimeSlice';

/**
 * The failure classes the UI has localized copy for (#4254 / #4459), keyed by
 * the camelCase form of the wire's PascalCase `class`. Any class not in this
 * set falls back to the English `causePlain` / `nextAction` on the payload.
 *
 * Shared with the legacy `ToolFailureLines` this replaces — same key set, same
 * i18n namespace, so no locale files change.
 */
const LOCALIZED_FAILURE_CLASSES: ReadonlySet<string> = new Set([
  'missingPermission',
  'missingApp',
  'serviceUnavailable',
  'badCredentials',
  'blockedByPolicy',
  'modelConnection',
  'timeout',
  'denied',
  'approvalExpired',
  'notFound',
  'unsupported',
]);

/** Lowercase the first character: `MissingPermission` → `missingPermission`. */
function toCamelClass(cls: string): string {
  return cls.length > 0 ? cls[0].toLowerCase() + cls.slice(1) : cls;
}

/**
 * A failed tool call, rendered through the vendored `tool-error` element
 * instead of the inline `ToolFailureLines` text it replaces.
 *
 * There is no retry telemetry for an arbitrary OpenHuman tool (the core does
 * not report an attempt count), so counters and Retry/Skip actions are omitted.
 * Retrying a
 * tool call is a re-send of the same turn, which belongs to whatever surface
 * offers "Try again" today (`aiRegenerate`), not to this card.
 */
export function ToolFailureCard({
  toolName,
  target,
  failure,
  diagnostic,
}: {
  toolName: string;
  /** Short context for the call, e.g. the server's display detail. Falls back to the failure class. */
  target?: string;
  failure: ToolFailureExplanation;
  /** Existing tool output, used when an unclassified failure has no useful summary. */
  diagnostic?: string;
}) {
  const { t } = useT();
  const camel = toCamelClass(failure.class);
  const known = LOCALIZED_FAILURE_CLASSES.has(camel) || camel === 'unknown';
  const cause =
    camel === 'unknown' && diagnostic?.trim()
      ? diagnostic.trim().split('\n').slice(0, 3).join('\n').slice(0, 360)
      : known
        ? t(`conversations.toolFailure.${camel}.cause`, failure.causePlain)
        : failure.causePlain;
  const next = known
    ? t(`conversations.toolFailure.${camel}.next`, failure.nextAction)
    : failure.nextAction;
  const why = t('conversations.toolFailure.whyLabel');
  const nextLabel = t('conversations.toolFailure.nextLabel');
  return (
    <ToolError
      data-testid="assistant-ui-tool-failure"
      name={toolName}
      target={
        target ?? (camel === 'unknown' ? t('conversations.tools.status.failed') : failure.class)
      }
      message={`${why}: ${cause} ${nextLabel}: ${next}`}
      retrying={false}
    />
  );
}
