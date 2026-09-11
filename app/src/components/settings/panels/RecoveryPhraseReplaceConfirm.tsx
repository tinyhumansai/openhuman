import { useT } from '../../../lib/i18n/I18nContext';
import { Alert } from '../../ui/Alert';
import Button from '../../ui/Button';

export interface RecoveryPhraseReplaceConfirmProps {
  onConfirmReplace: () => void;
  onImportInstead: () => void;
  onCancel: () => void;
}

/**
 * Inline "Replace wallet" confirmation gate. Deliberately NOT a modal
 * (Dialog/AlertDialog/ConfirmDialog): the panel already renders this as a
 * dedicated mode, and the e2e/unit specs locate its copy and buttons via
 * `screen.getByText` against the document — wrapping it in a portal-based
 * dialog would not change behavior but adds risk for no test-visible benefit.
 */
const RecoveryPhraseReplaceConfirm = ({
  onConfirmReplace,
  onImportInstead,
  onCancel,
}: RecoveryPhraseReplaceConfirmProps) => {
  const { t } = useT();

  return (
    <div className="space-y-5">
      <Alert variant="destructive" className="items-start p-4">
        <p className="text-sm leading-relaxed">{t('mnemonic.replaceWalletWarning')}</p>
      </Alert>

      <Button
        type="button"
        variant="primary"
        tone="danger"
        size="md"
        onClick={onConfirmReplace}
        className="w-full">
        {t('mnemonic.replaceWalletConfirm')}
      </Button>

      <Button type="button" variant="tertiary" onClick={onImportInstead} className="w-full">
        {t('mnemonic.alreadyHavePhrase')}
      </Button>

      <Button type="button" variant="tertiary" onClick={onCancel} className="w-full">
        {t('common.cancel')}
      </Button>
    </div>
  );
};

export default RecoveryPhraseReplaceConfirm;
