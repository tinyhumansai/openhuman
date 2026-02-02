import type { ConstitutionConfig } from '../constitution/types';
import type { ToolDefinition } from '../providers/interface';
import { buildConstitutionSection } from './sections/constitution';
import { buildContextSection, type UserContext } from './sections/context';
import {
  buildCryptoIntelligenceSection,
  type CryptoIntelligenceContext,
} from './sections/crypto-intelligence';
import { type AgentIdentity, buildIdentitySection } from './sections/identity';
import { buildMemoryRecallSection } from './sections/memory-recall';
import { buildSkillsSection } from './sections/skills';
import { buildToolsSection } from './sections/tools';

/** Minimal skill entry for prompt rendering */
interface SkillPromptEntry {
  name: string;
  description: string;
  location?: string;
}

/**
 * Parameters for building the full system prompt.
 */
export interface SystemPromptParams {
  /** Constitution config (ALWAYS loaded first) */
  constitution: ConstitutionConfig;
  /** Agent identity/persona */
  identity?: Partial<AgentIdentity>;
  /** Available tools */
  tools?: ToolDefinition[];
  /** Loaded skills */
  skills?: SkillPromptEntry[];
  /** User context (timezone, preferences, etc.) */
  userContext?: UserContext;
  /** Crypto market/portfolio context */
  cryptoContext?: CryptoIntelligenceContext;
  /** Prompt mode: full for main agent, minimal for sub-agents */
  mode?: 'full' | 'minimal' | 'none';
}

/**
 * Build the complete system prompt from modular sections.
 *
 * Section order (matches OpenClaw's architecture):
 * 1. Constitution (mandatory first — safety & compliance)
 * 2. Identity (who the agent is)
 * 3. Crypto Intelligence (domain knowledge)
 * 4. Tools (available function calls)
 * 5. Skills (available skill files)
 * 6. Memory Recall (how to search/write memory)
 * 7. User Context (preferences, timezone, project context)
 */
export function buildSystemPrompt(params: SystemPromptParams): string {
  const { mode = 'full' } = params;

  if (mode === 'none') {
    const name = params.identity?.name || 'AlphaHuman';
    return `You are ${name}, a crypto-native AI assistant.`;
  }

  const sections: string[] = [];

  // 1. Constitution (ALWAYS first)
  sections.push(buildConstitutionSection(params.constitution));

  // 2. Identity
  sections.push(buildIdentitySection(params.identity));

  // 3. Crypto Intelligence
  sections.push(buildCryptoIntelligenceSection(params.cryptoContext));

  if (mode === 'full') {
    // 4. Tools
    if (params.tools?.length) {
      sections.push(buildToolsSection(params.tools));
    }

    // 5. Skills
    if (params.skills?.length) {
      sections.push(buildSkillsSection(params.skills));
    }

    // 6. Memory Recall
    sections.push(buildMemoryRecallSection());
  }

  // 7. User Context (both modes)
  if (params.userContext) {
    sections.push(buildContextSection(params.userContext));
  }

  return sections.filter(Boolean).join('\n');
}
