/**
 * Agent identity / persona section of the system prompt.
 */

export interface AgentIdentity {
  name: string;
  tagline?: string;
  personality?: string;
  /** Custom identity markdown loaded from identity.md */
  customIdentity?: string;
}

const DEFAULT_IDENTITY: AgentIdentity = {
  name: 'AlphaHuman',
  tagline: 'Crypto-native AI assistant',
  personality:
    'You are precise, technical, and direct. You use proper crypto terminology and cite data sources. You never fabricate information.',
};

/**
 * Build the identity section of the system prompt.
 */
export function buildIdentitySection(identity: Partial<AgentIdentity> = {}): string {
  const id = { ...DEFAULT_IDENTITY, ...identity };
  const parts: string[] = [];

  parts.push('## Identity\n');
  parts.push(
    `You are **${id.name}**, a ${id.tagline || 'crypto-native AI assistant'} embedded in a crypto community platform.\n`
  );

  if (id.personality) {
    parts.push(id.personality);
    parts.push('');
  }

  if (id.customIdentity) {
    parts.push('### Custom Persona');
    parts.push(id.customIdentity);
    parts.push('');
  }

  return parts.join('\n');
}
