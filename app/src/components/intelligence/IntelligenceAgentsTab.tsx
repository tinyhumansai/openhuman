import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { useT } from '../../lib/i18n/I18nContext';
import type { AgentDefinitionDisplay } from '../../services/api/agentLibraryApi';
import { threadApi } from '../../services/api/threadApi';
import { chatSend } from '../../services/chatService';
import { selectActiveAgentProfileId } from '../../store/agentProfileSlice';
import { beginInferenceTurn, setToolTimelineForThread } from '../../store/chatRuntimeSlice';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import {
  loadThreadMessages,
  loadThreads,
  setActiveThread,
  setSelectedThread,
} from '../../store/threadSlice';
import type { ThreadMessage } from '../../types/thread';
import { chatThreadPath } from '../../utils/chatRoutes';
import AgentsLibraryPanel from './AgentsLibraryPanel';

const log = debug('intelligence:agents-tab');
const AGENT_TASK_THREAD_LABEL = 'tasks';
const CHAT_MODEL_ID = 'reasoning-v1';

function agentTaskThreadTitle(title: string): string {
  const trimmed = title.trim();
  const base = trimmed.length > 72 ? `${trimmed.slice(0, 69)}...` : trimmed;
  return `Agent task: ${base || 'Untitled task'}`;
}

function buildExplicitAgentPrompt(agent: AgentDefinitionDisplay, task: string): string {
  return [
    `@agent:${agent.id}`,
    '',
    'Run this task with the explicitly selected agent above. Treat the selected agent id as the user routing choice when resolving delegation ambiguity.',
    '',
    task.trim(),
  ].join('\n');
}

export default function IntelligenceAgentsTab() {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const navigate = useNavigate();
  const selectedAgentProfileId = useAppSelector(selectActiveAgentProfileId);
  const uiLocale = useAppSelector(state => state.locale?.current ?? 'en');
  const mountedRef = useRef(true);

  useEffect(() => {
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const [runningAgentId, setRunningAgentId] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  const handleRunAgentTask = useCallback(
    async (agent: AgentDefinitionDisplay, task: string) => {
      if (runningAgentId) return;
      setRunningAgentId(agent.id);
      setActionError(null);
      const now = new Date().toISOString();
      const launchPrompt = buildExplicitAgentPrompt(agent, task);
      const titleBase = task.trim().slice(0, 64) || agent.display_name;
      try {
        const thread = await threadApi.createNewThread([AGENT_TASK_THREAD_LABEL, 'agent-library']);
        await threadApi.updateTitle(thread.id, agentTaskThreadTitle(titleBase));
        const userMessage: ThreadMessage = {
          id: `msg_${globalThis.crypto?.randomUUID ? globalThis.crypto.randomUUID() : `${Date.now()}`}`,
          content: launchPrompt,
          type: 'text',
          extraMetadata: { source: 'agent-library', explicitAgentId: agent.id },
          sender: 'user',
          createdAt: now,
        };
        await threadApi.appendMessage(thread.id, userMessage);

        dispatch(setSelectedThread(thread.id));
        dispatch(setToolTimelineForThread({ threadId: thread.id, entries: [] }));
        dispatch(beginInferenceTurn({ threadId: thread.id }));
        dispatch(setActiveThread(thread.id));
        void dispatch(loadThreads());
        void dispatch(loadThreadMessages(thread.id));
        navigate(chatThreadPath(thread.id));

        await chatSend({
          threadId: thread.id,
          message: launchPrompt,
          model: CHAT_MODEL_ID,
          profileId: selectedAgentProfileId,
          locale: uiLocale,
        });
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        log('run explicit agent task failed agent=%s: %s', agent.id, msg);
        if (mountedRef.current) setActionError(t('intelligence.agents.runFailed'));
      } finally {
        if (mountedRef.current) setRunningAgentId(null);
      }
    },
    [dispatch, navigate, runningAgentId, selectedAgentProfileId, t, uiLocale]
  );

  return (
    <div className="space-y-4">
      {actionError && (
        <div className="rounded-xl border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-4 py-3 text-sm text-coral-700 dark:text-coral-300">
          {actionError}
        </div>
      )}
      <AgentsLibraryPanel onRunAgentTask={handleRunAgentTask} runningAgentId={runningAgentId} />
    </div>
  );
}
