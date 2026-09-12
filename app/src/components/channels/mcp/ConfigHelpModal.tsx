/**
 * ConfigHelpModal — a focused, roomy modal that hosts the configuration-help
 * chat (ConfigAssistantPanel) for one MCP server. Launched from the Connect
 * modal's "How do I get a token?" link and from the server detail page, so the
 * chat gets its own space instead of crowding the auth inputs.
 *
 * Built on the shared `ModalShell` primitive (Radix `Dialog` underneath), so it
 * gets a real focus trap and stacks correctly above the Connect modal via
 * Radix's own layering rather than a hand-picked z-index.
 */
import { useId } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { ModalShell } from '../../ui/ModalShell';
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
  const titleId = useId();

  // Fixed, server-specific research prompt the assistant auto-runs on open.
  const autoPrompt =
    `I'm connecting the MCP server "${displayName}" (${qualifiedName}).` +
    (description ? ` ${description}.` : '') +
    ` Walk me through, step by step, exactly where to obtain the credential I need:` +
    ` which website or dashboard, which account/settings page, and what scopes or permissions to enable,` +
    ` and the exact header name and value format to paste. Be concise and specific to this service.`;

  return (
    <ModalShell
      onClose={onClose}
      titleId={titleId}
      title={t('mcp.connectAuth.howToGetToken')}
      maxWidthClassName="max-w-2xl"
      contentClassName="flex h-[78vh] max-h-[88vh] min-h-0 flex-col px-5 py-4">
      <ConfigAssistantPanel
        qualifiedName={qualifiedName}
        autoPrompt={autoPrompt}
        onApplySuggestedEnv={onApplySuggestedEnv}
      />
    </ModalShell>
  );
};

export default ConfigHelpModal;
