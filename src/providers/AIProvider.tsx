import { createContext, type ReactNode, useContext, useEffect, useRef } from 'react';

import { loadConstitution } from '../lib/ai/constitution/loader';
import type { ConstitutionConfig } from '../lib/ai/constitution/types';
import { EntityManager } from '../lib/ai/entities/manager';
import { MemoryManager } from '../lib/ai/memory/manager';
import { CustomLLMProvider } from '../lib/ai/providers/custom';
import { type EmbeddingProvider, NullEmbeddingProvider } from '../lib/ai/providers/embeddings';
import type { LLMProvider } from '../lib/ai/providers/interface';
import { OpenAIEmbeddingProvider } from '../lib/ai/providers/openai';
import { SessionManager } from '../lib/ai/sessions/manager';
import { createMemoryReadTool } from '../lib/ai/tools/memory-read';
import { createMemorySearchTool } from '../lib/ai/tools/memory-search';
import { createMemoryWriteTool } from '../lib/ai/tools/memory-write';
import { ToolRegistry } from '../lib/ai/tools/registry';
import { createWebSearchTool } from '../lib/ai/tools/web-search';
import { bridgeSkillTools } from '../lib/skills/tool-bridge';
import type { SkillState } from '../lib/skills/types';
import { setAIError, setAIStatus, setMemoryInitialized } from '../store/aiSlice';
import { useAppDispatch, useAppSelector } from '../store/hooks';

/** AI context value */
interface AIContextValue {
  memoryManager: MemoryManager;
  sessionManager: SessionManager;
  toolRegistry: ToolRegistry;
  entityManager: EntityManager;
  llmProvider: LLMProvider | null;
  embeddingProvider: EmbeddingProvider;
  constitution: ConstitutionConfig | null;
  isReady: boolean;
}

const AIContext = createContext<AIContextValue | null>(null);

export function useAI(): AIContextValue {
  const ctx = useContext(AIContext);
  if (!ctx) {
    throw new Error('useAI must be used within an AIProvider');
  }
  return ctx;
}

export default function AIProvider({ children }: { children: ReactNode }) {
  const dispatch = useAppDispatch();
  const { config } = useAppSelector(state => state.ai);
  const { token } = useAppSelector(state => state.auth);
  const skillsMap = useAppSelector(state => state.skills.skills);

  const memoryManagerRef = useRef(new MemoryManager());
  const sessionManagerRef = useRef(new SessionManager());
  const toolRegistryRef = useRef(new ToolRegistry());
  const entityManagerRef = useRef(new EntityManager());
  const constitutionRef = useRef<ConstitutionConfig | null>(null);
  const llmProviderRef = useRef<LLMProvider | null>(null);
  const embeddingProviderRef = useRef<EmbeddingProvider>(new NullEmbeddingProvider());
  const isReadyRef = useRef(false);

  useEffect(() => {
    if (!token) return;

    let cancelled = false;

    async function initAI() {
      dispatch(setAIStatus('initializing'));

      try {
        // 1. Load constitution
        const constitution = await loadConstitution();
        if (cancelled) return;
        constitutionRef.current = constitution;

        // 2. Initialize memory system
        await memoryManagerRef.current.init();
        if (cancelled) return;
        dispatch(setMemoryInitialized(true));

        // 3. Initialize entity database
        await entityManagerRef.current.init();
        if (cancelled) return;

        // 4. Setup embedding provider
        if (config.openaiApiKey) {
          const provider = new OpenAIEmbeddingProvider({
            id: 'openai',
            apiKey: config.openaiApiKey,
          });
          embeddingProviderRef.current = provider;
          memoryManagerRef.current.setEmbeddingProvider(provider);
        }

        // 5. Setup LLM provider
        if (config.llmEndpoint) {
          llmProviderRef.current = new CustomLLMProvider({
            id: 'custom',
            endpoint: config.llmEndpoint,
            model: config.llmModel,
          });
        }

        // 6. Index memory files
        await memoryManagerRef.current.indexAll();
        if (cancelled) return;

        // 7. Initialize sessions
        await sessionManagerRef.current.init();
        if (cancelled) return;

        // 8. Register tools
        const toolReg = toolRegistryRef.current;
        toolReg.register(createMemorySearchTool(memoryManagerRef.current));
        toolReg.register(createMemoryReadTool(memoryManagerRef.current));
        toolReg.register(createMemoryWriteTool(memoryManagerRef.current, constitution));
        toolReg.register(
          createWebSearchTool({
            endpoint: config.webSearchEndpoint,
            apiKey: config.webSearchApiKey,
          })
        );

        isReadyRef.current = true;
        dispatch(setAIStatus('ready'));
      } catch (error) {
        if (!cancelled) {
          const msg = error instanceof Error ? error.message : String(error);
          dispatch(setAIError(msg));
        }
      }
    }

    initAI();

    return () => {
      cancelled = true;
    };
  }, [token, config, dispatch]);

  // Register/unregister skill tools when skill statuses change
  const registeredSkillToolsRef = useRef<Set<string>>(new Set());
  useEffect(() => {
    const toolReg = toolRegistryRef.current;
    const currentlyRegistered = registeredSkillToolsRef.current;
    const newRegistered = new Set<string>();

    for (const [skillId, skill] of Object.entries(skillsMap) as [string, SkillState][]) {
      if (skill.status === 'ready' && skill.tools.length > 0) {
        const bridged = bridgeSkillTools(skillId, skill.tools);
        for (const bt of bridged) {
          newRegistered.add(bt.name);
          if (!currentlyRegistered.has(bt.name)) {
            toolReg.register({
              definition: { name: bt.name, description: bt.description, parameters: bt.parameters },
              execute: async args => ({ content: await bt.execute(args) }),
            });
          }
        }
      }
    }

    // Unregister tools from skills that are no longer ready
    for (const name of currentlyRegistered) {
      if (!newRegistered.has(name)) {
        toolReg.unregister(name);
      }
    }

    registeredSkillToolsRef.current = newRegistered;
  }, [skillsMap]);

  const contextValue: AIContextValue = {
    memoryManager: memoryManagerRef.current,
    sessionManager: sessionManagerRef.current,
    toolRegistry: toolRegistryRef.current,
    entityManager: entityManagerRef.current,
    llmProvider: llmProviderRef.current,
    embeddingProvider: embeddingProviderRef.current,
    constitution: constitutionRef.current,
    isReady: isReadyRef.current,
  };

  return <AIContext.Provider value={contextValue}>{children}</AIContext.Provider>;
}
