/**
 * Memory → Settings → Erase all memory: the one control that wipes the user's
 * entire memory (`memory_erase_all`). On the hosted engine that is the
 * account's whole hosted memory; on a CortexDB reached directly it is the
 * user's own subtree. Nothing erased comes back, so the action sits behind an
 * `AlertDialog` (which does not dismiss on an outside click) and an explicit
 * "I understand" checkbox, mirroring Settings → Account's clear-app-data wipe.
 *
 * Failures are mapped from the core's structured error code to translated
 * text; raw server detail is never shown.
 *
 * debug logging: DEBUG=openhuman:memory:erase
 */
import debug from 'debug';
import { Trash2 } from 'lucide-react';
import { useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { memoryEraseAll, memoryErrorCode } from '../../services/api/memoryApi';
import { trackAnalyticsEvent } from '../analytics';
import {
  Alert,
  AlertDescription,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogRoot,
  AlertDialogTitle,
  Button,
  Card,
  Checkbox,
  Field,
} from '../ui';
import { Spinner } from '../ui/icons';
import { toast } from '../ui/Toast';

const log = debug('openhuman:memory:erase');

/** Translation key for a failed erase, by the core's structured error code. */
export function eraseErrorKey(err: unknown): string {
  switch (memoryErrorCode(err)) {
    case 'UNSUPPORTED':
      return 'memoryPage.settings.eraseError.unsupported';
    case 'INSUFFICIENT_CREDITS':
      return 'memoryPage.settings.eraseError.insufficientCredits';
    case 'PLAN_REQUIRED':
      return 'memory.error.planRequired';
    case 'MEMORY_OFF':
      return 'memoryPage.settings.eraseError.memoryOff';
    case 'UNAVAILABLE':
      return 'memoryPage.settings.eraseError.unavailable';
    case 'UNAUTHORIZED':
      return 'memoryPage.settings.eraseError.unauthorized';
    default:
      return 'memoryPage.settings.eraseError.generic';
  }
}

interface MemoryEraseAllCardProps {
  /** Memory is off: the control is shown disabled. */
  disabled?: boolean;
  /** Called after a successful erase so the page can re-read memory state. */
  onErased?: () => void | Promise<void>;
}

export default function MemoryEraseAllCard({
  disabled = false,
  onErased,
}: MemoryEraseAllCardProps) {
  const { t } = useT();
  const [open, setOpen] = useState(false);
  const [acknowledged, setAcknowledged] = useState(false);
  const [erasing, setErasing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const changeOpen = (next: boolean) => {
    if (erasing) return;
    setOpen(next);
    if (!next) {
      setAcknowledged(false);
      setError(null);
    }
  };

  const erase = async () => {
    if (!acknowledged || erasing) return;
    setErasing(true);
    setError(null);
    let erased = false;
    try {
      const result = await memoryEraseAll();
      log('erase_all ok: erased_scopes=%d', result?.erased_scopes ?? 0);
      erased = true;
    } catch (err) {
      const key = eraseErrorKey(err);
      log('erase_all failed: code=%s', memoryErrorCode(err) ?? 'none');
      setError(t(key));
    } finally {
      setErasing(false);
    }
    // The erase has already succeeded; reporting or refresh failures must not
    // read as a failed erase.
    if (erased) {
      try {
        trackAnalyticsEvent('memory_erased_all');
        toast.add({
          type: 'success',
          title: t('memoryPage.settings.erasedToast'),
          description: t('memoryPage.settings.erasedToastBody'),
        });
        setOpen(false);
        setAcknowledged(false);
      } catch (err) {
        log('erase_all success reporting failed: %s', err instanceof Error ? err.name : 'unknown');
      }
      try {
        await onErased?.();
      } catch (err) {
        log('onErased callback failed: %s', err instanceof Error ? err.name : 'unknown');
      }
    }
  };

  return (
    <>
      <Card title={t('memoryPage.settings.eraseTitle')} data-testid="memory-erase-card">
        <Field
          label={t('memoryPage.settings.eraseAction')}
          description={t('memoryPage.settings.eraseDescription')}
          control={
            <Button
              variant="secondary"
              tone="danger"
              size="sm"
              leadingIcon={<Trash2 className="h-3.5 w-3.5" aria-hidden />}
              disabled={disabled}
              analyticsId="memory-erase-all-open"
              data-testid="memory-erase-open"
              onClick={() => setOpen(true)}>
              {t('memoryPage.settings.eraseAction')}
            </Button>
          }
        />
      </Card>

      <AlertDialogRoot open={open} onOpenChange={changeOpen}>
        <AlertDialogContent className="max-w-md" data-testid="memory-erase-dialog">
          <AlertDialogTitle>{t('memoryPage.settings.eraseConfirmTitle')}</AlertDialogTitle>
          <AlertDialogDescription asChild>
            <div className="space-y-3 leading-relaxed">
              <p>{t('memoryPage.settings.eraseConfirmBody')}</p>
              <p className="font-medium text-coral-600 dark:text-coral-300">
                {t('memoryPage.settings.eraseIrreversible')}
              </p>
            </div>
          </AlertDialogDescription>
          <label
            htmlFor="memory-erase-ack"
            className="mt-4 flex items-start gap-2 text-sm text-content">
            <Checkbox
              id="memory-erase-ack"
              data-testid="memory-erase-ack"
              checked={acknowledged}
              disabled={erasing}
              onCheckedChange={setAcknowledged}
            />
            <span>{t('memoryPage.settings.eraseConfirmCheck')}</span>
          </label>
          {error && (
            <Alert
              variant="destructive"
              density="compact"
              className="mt-4"
              data-testid="memory-erase-error">
              <AlertDescription>{error}</AlertDescription>
            </Alert>
          )}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={erasing}>{t('common.cancel')}</AlertDialogCancel>
            <AlertDialogAction
              disabled={!acknowledged || erasing}
              data-testid="memory-erase-confirm"
              onClick={event => {
                // Keep the dialog open while erasing so progress and any
                // failure stay visible; success closes it.
                event.preventDefault();
                void erase();
              }}>
              {erasing && <Spinner />}
              {erasing ? t('memoryPage.settings.erasing') : t('memoryPage.settings.eraseConfirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialogRoot>
    </>
  );
}
