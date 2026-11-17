/**
 * ScanBlockedDialog
 * -----------------
 *
 * Shown when an install comes back `scan_blocked`: the core fetched and
 * scanned the SKILL.md twice and the supply-chain scan still blocks it, so
 * nothing was written. The user chooses:
 *
 *   - "Block install" (default, focused): keep it uninstalled. Escape, the
 *     backdrop and the close button mean the same.
 *   - "Install anyway": the caller re-runs the install with
 *     `acknowledgedDigest` set to the blocked document's digest, so only
 *     the document shown here can install.
 */
import debug from 'debug';

import { useT } from '../../lib/i18n/I18nContext';
import type { ScanBlocked } from '../../services/api/skillRegistryApi';
import { ModalShell } from '../ui';
import Button from '../ui/Button';

const log = debug('skills:scan-blocked-dialog');

interface Props {
  skillName: string;
  scan: ScanBlocked;
  installing: boolean;
  error?: string | null;
  onBlock: () => void;
  onInstallAnyway: () => void;
}

export default function ScanBlockedDialog({
  skillName,
  scan,
  installing,
  error,
  onBlock,
  onInstallAnyway,
}: Props) {
  const { t } = useT();

  const block = () => {
    if (installing) return;
    log('block: name=%s findings=%d', skillName, scan.findings.length);
    onBlock();
  };

  const installAnyway = () => {
    log('install-anyway: name=%s findings=%d', skillName, scan.findings.length);
    onInstallAnyway();
  };

  return (
    <ModalShell
      onClose={block}
      title={t('skills.scan.title')}
      titleId="scan-blocked-title"
      describedBy="scan-blocked-description"
      maxWidthClassName="max-w-[480px]"
      contentClassName="max-h-[60vh] overflow-y-auto px-5 py-4"
      closePolicy={installing ? { escape: false, backdrop: false, button: false } : undefined}
      testId="scan-blocked-dialog"
      footer={
        <div className="flex items-center justify-end gap-2">
          <Button
            variant="secondary"
            tone="danger"
            size="sm"
            disabled={installing}
            onClick={installAnyway}
            data-testid="scan-blocked-install-anyway">
            {installing ? t('skills.install.installing') : t('skills.scan.installAnyway')}
          </Button>
          <Button
            variant="primary"
            size="sm"
            autoFocus
            disabled={installing}
            onClick={block}
            data-testid="scan-blocked-block">
            {t('skills.scan.block')}
          </Button>
        </div>
      }>
      <p id="scan-blocked-description" className="text-sm text-content-secondary">
        {t('skills.scan.description').replace('{name}', skillName)}
      </p>
      <p className="mt-3 text-xs font-medium text-content-secondary">
        {t('skills.scan.findingsLabel')}
      </p>
      <ul className="mt-1 space-y-1.5" data-testid="scan-blocked-findings">
        {scan.findings.map((finding, index) => (
          <li
            key={`${finding.check}-${index}`}
            className="flex items-start gap-2 rounded-lg bg-surface-muted px-3 py-2 text-xs text-content-secondary">
            <span
              className={
                finding.verdict === 'block'
                  ? 'shrink-0 rounded bg-coral-50 px-1.5 py-0.5 font-medium text-coral-700'
                  : 'shrink-0 rounded bg-amber-50 px-1.5 py-0.5 font-medium text-amber-700'
              }>
              {finding.verdict === 'block'
                ? t('skills.scan.verdictBlock')
                : t('skills.scan.verdictWarn')}
            </span>
            <span className="wrap-break-word">{finding.message}</span>
          </li>
        ))}
      </ul>
      <p className="mt-3 text-xs text-content-muted">{t('skills.scan.warning')}</p>
      {error ? (
        <div
          role="alert"
          className="mt-3 rounded-lg border border-coral-200 bg-coral-50 px-3 py-2 text-xs text-coral-700">
          {error}
        </div>
      ) : null}
    </ModalShell>
  );
}
