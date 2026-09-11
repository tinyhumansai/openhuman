/*
 * "Test model" result banner for the custom-routing dialog — shown while the
 * test is running, and after it succeeds or fails.
 *
 * Built on the `Alert` primitive so the three states reuse the app's semantic
 * tones (destructive / warning / success) instead of three hand-written
 * coral/amber/sage class strings that had to be kept in sync by hand.
 *
 * The explicit `role` is preserved rather than left to `Alert`'s default.
 * `Alert` promotes `warning` to an assertive live region, which is right for a
 * warning that arrives unbidden — but the warning tone here means "the test is
 * still running", and interrupting a screen reader to say so is exactly what a
 * polite `status` region is for. `...rest` wins over the primitive's default,
 * so passing `role` is all that is needed.
 */
import { useT } from '../../../../lib/i18n/I18nContext';
import Alert from '../../../ui/Alert';
import { formatI18n } from './aiPanelTypes';

export const ModelTestResultPanel = ({
  testBusy,
  testReply,
  testError,
  testStartedAt,
  currentProviderString,
}: {
  testBusy: boolean;
  testReply: string | null;
  testError: string | null;
  testStartedAt: string | null;
  currentProviderString: string | null;
}) => {
  const { t } = useT();
  if (!testBusy && !testReply && !testError && !testStartedAt) return null;

  const variant = testError ? 'destructive' : testBusy ? 'warning' : 'success';

  return (
    <Alert
      variant={variant}
      role={testError ? 'alert' : 'status'}
      className="flex-col gap-0 px-3 py-2 text-xs">
      <div className="font-semibold">
        {testError
          ? t('settings.ai.testFailed')
          : testBusy
            ? t('settings.ai.testingModel')
            : t('settings.ai.modelResponse')}
      </div>
      <div className="mt-1 flex flex-col gap-1">
        <div className="font-mono text-[11px] text-current/80">
          {formatI18n(t('settings.ai.providerWithValue'), {
            value: currentProviderString ?? t('settings.ai.noneDash'),
          })}
        </div>
        <div className="font-mono text-[11px] text-current/80">
          {t('settings.ai.promptHelloWorld')}
        </div>
        {testStartedAt && (
          <div className="font-mono text-[11px] text-current/80">
            {formatI18n(t('settings.ai.startedAt'), { value: testStartedAt })}
          </div>
        )}
      </div>
      {testBusy ? (
        <div className="mt-2 rounded-md border border-current/15 bg-surface/70 px-3 py-2 text-[12px]">
          {t('settings.ai.waitingForModelResponse')}
        </div>
      ) : testError ? (
        <div className="mt-2 rounded-md border border-current/15 bg-surface/70 px-3 py-2 font-mono text-[11px] whitespace-pre-wrap wrap-break-word">
          {testError}
        </div>
      ) : (
        <div className="mt-3 flex flex-col gap-1.5">
          <div className="text-[11px] font-semibold uppercase tracking-wide text-current/80">
            {t('settings.ai.response')}
          </div>
          <div className="rounded-md border border-current/15 bg-surface/70 px-3 py-3 text-[13px] leading-relaxed text-content whitespace-pre-wrap wrap-break-word">
            {testReply}
          </div>
        </div>
      )}
    </Alert>
  );
};

export default ModelTestResultPanel;
