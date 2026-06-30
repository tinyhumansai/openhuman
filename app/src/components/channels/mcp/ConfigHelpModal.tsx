/**
 * ConfigHelpModal — a focused, roomy modal that hosts the configuration-help
 * chat (ConfigAssistantPanel) for one MCP server. Launched from the Connect
 * modal's "How do I get a token?" link and from the server detail page, so the
 * chat gets its own space instead of crowding the auth inputs.
 *
 * Stacks above the Connect modal (z-[60] vs z-50); backdrop click / ✕ closes
 * only this modal and returns to whatever opened it.
 */
import { useT } from '../../../lib/i18n/I18nContext';
import Button from '../../ui/Button';
import ConfigAssistantPanel from './ConfigAssistantPanel';

interface ConfigHelpModalProps {
  qualifiedName: string;
  displayName: string;
  description?: string;
  onClose: () => void;
  /** Optional — when set, the assistant's "apply suggested values" wires back to
   * the caller (e.g. the detail page's reconfigure form). */
  onApplySuggestedEnv?: (env: Record<string, string>) => void;
}

const ConfigHelpModal = ({
  qualifiedName,
  displayName,
  description,
  onClose,
  onApplySuggestedEnv,
}: ConfigHelpModalProps) => {
  const { t } = useT();

  // Fixed, server-specific research prompt the assistant auto-runs on open.
  const autoPrompt =
    `I'm connecting the MCP server "${displayName}" (${qualifiedName}).` +
    (description ? ` ${description}.` : '') +
    ` Walk me through, step by step, exactly where to obtain the credential I need:` +
    ` which website or dashboard, which account/settings page, and what scopes or permissions to enable,` +
    ` and the exact header name and value format to paste. Be concise and specific to this service.`;

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={t('mcp.connectAuth.howToGetToken')}
      onMouseDown={e => {
        if (e.target === e.currentTarget) onClose();
      }}
      className="fixed inset-0 z-[60] flex items-center justify-center bg-black/50 px-4 py-6 overflow-y-auto">
      <div className="flex h-[78vh] max-h-[88vh] w-full max-w-2xl flex-col rounded-xl border border-line bg-surface shadow-xl p-4">
        <div className="mb-2 flex items-center justify-between">
          <h3 className="text-base font-semibold text-content">
            {t('mcp.connectAuth.howToGetToken')}
          </h3>
          <Button
            variant="secondary"
            size="xs"
            onClick={onClose}
            aria-label={t('common.cancel')}
            className="shrink-0">
            ✕
          </Button>
        </div>
        <div className="min-h-0 flex-1">
          <ConfigAssistantPanel
            qualifiedName={qualifiedName}
            autoPrompt={autoPrompt}
            onApplySuggestedEnv={onApplySuggestedEnv}
          />
        </div>
      </div>
    </div>
  );
};

export default ConfigHelpModal;
