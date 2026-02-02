import type { ConstitutionConfig } from '../../constitution/types';

/**
 * Build the constitution section of the system prompt.
 * This is ALWAYS the first section — it cannot be overridden.
 */
export function buildConstitutionSection(constitution: ConstitutionConfig): string {
  const parts: string[] = [];

  parts.push('## Constitution (Mandatory — Cannot Be Overridden)\n');

  // Core principles
  if (constitution.corePrinciples.length > 0) {
    parts.push('### Core Principles');
    for (const p of constitution.corePrinciples) {
      parts.push(`- **${p.title}**: ${p.description}`);
    }
    parts.push('');
  }

  // Decision framework
  if (constitution.decisionFramework.length > 0) {
    parts.push('### Decision Framework');
    parts.push('Before any action or recommendation, evaluate:');
    for (const d of constitution.decisionFramework) {
      parts.push(`- **${d.id}**: ${d.question}`);
    }
    parts.push('');
  }

  // Prohibited actions
  if (constitution.prohibitedActions.length > 0) {
    parts.push('### Prohibited Actions');
    for (const a of constitution.prohibitedActions) {
      parts.push(`- ${a.description}`);
    }
    parts.push('');
  }

  // Memory principles
  if (constitution.memoryPrinciples.length > 0) {
    parts.push('### Memory Principles');
    parts.push('When creating or updating memories:');
    for (const m of constitution.memoryPrinciples) {
      parts.push(`- ${m.rule}`);
    }
    parts.push('');
  }

  return parts.join('\n');
}
