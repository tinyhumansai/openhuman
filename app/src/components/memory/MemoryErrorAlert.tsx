/**
 * How a failed memory action is shown. An exhausted credit balance
 * (`INSUFFICIENT_CREDITS`) is a prompt to top up, and memory missing from the
 * account's plan (`PLAN_REQUIRED`, #8081) a prompt to upgrade, not errors: each
 * explains itself and links to billing. Every other failure keeps the
 * destructive alert.
 */
import { useNavigate } from 'react-router-dom';

import { useT } from '../../lib/i18n/I18nContext';
import { isOutOfCreditsMessage, isPlanRequiredMessage } from '../../services/api/memoryApi';
import { Alert, AlertDescription, AlertTitle, Button } from '../ui';

/** Where the app sends every "top up" action (`open_billing` in useAppNotices). */
const BILLING_ROUTE = '/settings/account';

interface MemoryCreditsPromptProps {
  /** The out-of-credits explanation. */
  message: string;
  /** Retries the failed action (e.g. after a top-up), shown beside Top up. */
  onRetry?: () => void;
  className?: string;
  'data-testid'?: string;
}

/** A billing refusal and how to lift it. */
const BILLING_PROMPTS = {
  'out-of-credits': {
    title: 'memory.outOfCredits.title',
    action: 'memory.outOfCredits.action',
    actionTestId: 'memory-top-up',
  },
  'plan-required': {
    title: 'memory.planRequired.title',
    action: 'memory.planRequired.action',
    actionTestId: 'memory-upgrade',
  },
} as const;

/** A billing refusal's explanation, with a button to billing. */
function MemoryBillingPrompt({
  kind,
  message,
  onRetry,
  className,
  'data-testid': testId,
}: MemoryCreditsPromptProps & { kind: keyof typeof BILLING_PROMPTS }) {
  const { t } = useT();
  const navigate = useNavigate();
  const prompt = BILLING_PROMPTS[kind];
  return (
    <Alert variant="warning" className={className} data-testid={testId} data-kind={kind}>
      <AlertTitle>{t(prompt.title)}</AlertTitle>
      <AlertDescription className="flex flex-wrap items-center gap-2">
        <span>{message}</span>
        <Button
          type="button"
          size="xs"
          variant="primary"
          data-testid={prompt.actionTestId}
          onClick={() => navigate(BILLING_ROUTE)}>
          {t(prompt.action)}
        </Button>
        {onRetry && <RetryButton onRetry={onRetry} />}
      </AlertDescription>
    </Alert>
  );
}

/** "Out of credits: top up", with a button to billing. */
export function MemoryCreditsPrompt(props: MemoryCreditsPromptProps) {
  return <MemoryBillingPrompt kind="out-of-credits" {...props} />;
}

interface MemoryErrorAlertProps {
  /** The message `memoryErrorMessage(err, t)` produced. */
  message: string;
  className?: string;
  'data-testid'?: string;
  /** Retries the failed action; shown in both the prompt and the error alert. */
  onRetry?: () => void;
}

function RetryButton({ onRetry }: { onRetry: () => void }) {
  const { t } = useT();
  return (
    <Button type="button" variant="tertiary" size="xs" onClick={onRetry}>
      {t('common.retry')}
    </Button>
  );
}

/** The billing prompt for an exhausted balance or a plan without memory, else
 * the error alert. */
export default function MemoryErrorAlert({
  message,
  className,
  'data-testid': testId,
  onRetry,
}: MemoryErrorAlertProps) {
  const { t } = useT();
  const billing = isOutOfCreditsMessage(message, t)
    ? 'out-of-credits'
    : isPlanRequiredMessage(message, t)
      ? 'plan-required'
      : null;
  if (billing) {
    return (
      <MemoryBillingPrompt
        kind={billing}
        message={message}
        onRetry={onRetry}
        className={className}
        data-testid={testId}
      />
    );
  }
  return (
    <Alert variant="destructive" className={className} data-testid={testId}>
      <AlertDescription>
        {message}
        {onRetry && (
          <>
            {' '}
            <RetryButton onRetry={onRetry} />
          </>
        )}
      </AlertDescription>
    </Alert>
  );
}
