import { useT } from '../../../lib/i18n/I18nContext';
import type { ToolFailureExplanation } from '../../../store/chatRuntimeSlice';

/**
 * The failure classes the UI has localized copy for (#4254 / #4459), keyed by
 * the camelCase form of the wire's PascalCase `class`. Any class not in this
 * set falls back to the English `causePlain` / `nextAction` on the payload.
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
  'unknown',
]);

/** Lowercase the first character: `MissingPermission` → `missingPermission`. */
function toCamelClass(cls: string): string {
  return cls.length > 0 ? cls[0].toLowerCase() + cls.slice(1) : cls;
}

/**
 * The "why + what to do next" pair rendered under a failed tool row (#4254 /
 * #4459). Copy resolves by failure class from i18n, falling back to the English
 * `causePlain` / `nextAction` carried on the wire when the class is one the UI
 * hasn't localized. Shared by the parent processing transcript and the
 * sub-agent renderers so a failed child tool shows the same why/next copy.
 */
export function ToolFailureLines({ failure }: { failure: ToolFailureExplanation }) {
  const { t } = useT();
  const camel = toCamelClass(failure.class);
  const known = LOCALIZED_FAILURE_CLASSES.has(camel);
  const cause = known
    ? t(`conversations.toolFailure.${camel}.cause`, failure.causePlain)
    : failure.causePlain;
  const next = known
    ? t(`conversations.toolFailure.${camel}.next`, failure.nextAction)
    : failure.nextAction;
  return (
    <span
      data-testid="processing-tool-failure"
      className="mt-1 flex flex-col gap-0.5 text-[11px] leading-snug">
      <span className="text-coral-600 dark:text-coral-300">
        <span className="font-semibold">{t('conversations.toolFailure.whyLabel')}:</span> {cause}
      </span>
      <span className="text-content-muted">
        <span className="font-semibold">{t('conversations.toolFailure.nextLabel')}:</span> {next}
      </span>
    </span>
  );
}
