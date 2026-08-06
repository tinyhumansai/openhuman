import { useLocation, useNavigate } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import ChipTabs from '../../layout/ChipTabs';
import PageSectionHeader from '../../layout/PageSectionHeader';
import AgentChatPanel from './AgentChatPanel';
import AIPanel from './AIPanel';
import LocalModelDebugPanel from './LocalModelDebugPanel';

type LlmChip = 'api-keys' | 'local-model' | 'agent-chat';

const LLM_CHIPS: readonly LlmChip[] = ['api-keys', 'local-model', 'agent-chat'];

/**
 * The Connections → LLM surface as a three-chip page:
 *   - **API keys** — the main AI provider / model configuration (AIPanel).
 *   - **Local Model Debug** — local runtime status + per-capability testers.
 *   - **Agent Chat Debug** — the raw agent-chat tester.
 *
 * Local Model Debug and Agent Chat used to be standalone Developer Options
 * pages; they're folded in here so everything LLM-related lives on one page.
 * The active chip is hash-backed so legacy diagnostics deep links select the
 * intended surface and chip selection remains shareable.
 *
 * Each chip renders its underlying panel unembedded so it keeps the same
 * PanelPage chrome + `p-4` padding as the sibling Connections tabs (Voice,
 * Embeddings, …); the two-pane shell hides the redundant back button.
 */
const LlmConnectionsPanel = () => {
  const { t } = useT();
  const location = useLocation();
  const navigate = useNavigate();
  const requestedChip = location.hash.slice(1);
  const chip: LlmChip = LLM_CHIPS.includes(requestedChip as LlmChip)
    ? (requestedChip as LlmChip)
    : 'api-keys';

  const setChip = (next: LlmChip) => {
    navigate(
      { pathname: location.pathname, search: location.search, hash: `#${next}` },
      { replace: true }
    );
  };

  return (
    <div className="flex h-full flex-col gap-4">
      <PageSectionHeader
        title={t('pages.settings.ai.llm')}
        description={t('connections.header.llm')}
        tabs={
          <ChipTabs<LlmChip>
            ariaLabel={t('pages.settings.ai.llm')}
            testIdPrefix="llm-chip"
            className="inline-flex flex-wrap items-center gap-1.5"
            value={chip}
            onChange={setChip}
            items={[
              { id: 'api-keys', label: t('connections.llm.apiKeys') },
              { id: 'local-model', label: t('settings.developerMenu.localModelDebug.title') },
              { id: 'agent-chat', label: t('settings.developerMenu.agentChat.title') },
            ]}
          />
        }
      />
      <div className="min-h-0 flex-1 overflow-hidden rounded-2xl border border-line bg-surface shadow-subtle">
        {chip === 'api-keys' && <AIPanel />}
        {chip === 'local-model' && <LocalModelDebugPanel />}
        {chip === 'agent-chat' && <AgentChatPanel />}
      </div>
    </div>
  );
};

export default LlmConnectionsPanel;
