import { describe, expect, it } from 'vitest';

import type { ConstitutionConfig } from '../../constitution/types';
import { buildConstitutionSection } from '../sections/constitution';
import { buildContextSection } from '../sections/context';
import { buildCryptoIntelligenceSection } from '../sections/crypto-intelligence';
import { buildIdentitySection } from '../sections/identity';
import { buildMemoryRecallSection } from '../sections/memory-recall';
import { buildSkillsSection } from '../sections/skills';
import { buildToolsSection } from '../sections/tools';

const mockConstitution: ConstitutionConfig = {
  raw: '',
  corePrinciples: [{ id: 'p1', title: 'Safety First', description: 'Protect the user.' }],
  memoryPrinciples: [{ rule: 'Store facts only' }],
  decisionFramework: [{ id: 'safety', question: 'Is this safe?' }],
  prohibitedActions: [{ description: 'No trading without consent' }],
  interactionGuidelines: ['Be precise'],
  isDefault: true,
};

describe('buildConstitutionSection', () => {
  it('should include header', () => {
    const result = buildConstitutionSection(mockConstitution);
    expect(result).toContain('Constitution');
    expect(result).toContain('Cannot Be Overridden');
  });

  it('should include principles', () => {
    const result = buildConstitutionSection(mockConstitution);
    expect(result).toContain('Safety First');
    expect(result).toContain('Protect the user.');
  });

  it('should include prohibited actions', () => {
    const result = buildConstitutionSection(mockConstitution);
    expect(result).toContain('No trading without consent');
  });

  it('should include memory principles', () => {
    const result = buildConstitutionSection(mockConstitution);
    expect(result).toContain('Store facts only');
  });

  it('should handle empty constitution', () => {
    const empty: ConstitutionConfig = {
      raw: '',
      corePrinciples: [],
      memoryPrinciples: [],
      decisionFramework: [],
      prohibitedActions: [],
      interactionGuidelines: [],
      isDefault: true,
    };
    const result = buildConstitutionSection(empty);
    expect(result).toContain('Constitution');
  });
});

describe('buildIdentitySection', () => {
  it('should use default identity', () => {
    const result = buildIdentitySection();
    expect(result).toContain('AlphaHuman');
    expect(result).toContain('Crypto-native AI assistant');
  });

  it('should use custom identity', () => {
    const result = buildIdentitySection({ name: 'CryptoBot', tagline: 'Your trading companion' });
    expect(result).toContain('CryptoBot');
    expect(result).toContain('trading companion');
  });

  it('should include custom persona when provided', () => {
    const result = buildIdentitySection({ customIdentity: 'Always speak like a pirate.' });
    expect(result).toContain('Custom Persona');
    expect(result).toContain('pirate');
  });
});

describe('buildCryptoIntelligenceSection', () => {
  it('should include domain knowledge areas', () => {
    const result = buildCryptoIntelligenceSection();
    expect(result).toContain('DeFi protocols');
    expect(result).toContain('On-chain analytics');
    expect(result).toContain('Trading concepts');
    expect(result).toContain('Token economics');
    expect(result).toContain('Cross-chain');
    expect(result).toContain('Security');
  });

  it('should include market summary when provided', () => {
    const result = buildCryptoIntelligenceSection({ marketSummary: 'BTC dominance at 52%' });
    expect(result).toContain('BTC dominance at 52%');
  });

  it('should include active chains when provided', () => {
    const result = buildCryptoIntelligenceSection({ activeChains: ['Ethereum', 'Arbitrum'] });
    expect(result).toContain('Ethereum, Arbitrum');
  });
});

describe('buildMemoryRecallSection', () => {
  it('should include search instructions', () => {
    const result = buildMemoryRecallSection();
    expect(result).toContain('memory_search');
    expect(result).toContain('memory_read');
  });

  it('should reference memory file paths', () => {
    const result = buildMemoryRecallSection();
    expect(result).toContain('memory.md');
    expect(result).toContain('memory/preferences.md');
    expect(result).toContain('memory/portfolio.md');
  });

  it('should include constitutional memory principles', () => {
    const result = buildMemoryRecallSection();
    expect(result).toContain('Constitutional Memory Principles');
  });
});

describe('buildSkillsSection', () => {
  it('should return empty string for no skills', () => {
    expect(buildSkillsSection([])).toBe('');
  });

  it('should format skills as XML', () => {
    const result = buildSkillsSection([
      { name: 'test-skill', description: 'A test skill', location: '/path/to/skill' },
    ]);
    expect(result).toContain('<available_skills>');
    expect(result).toContain('<name>test-skill</name>');
    expect(result).toContain('<description>A test skill</description>');
    expect(result).toContain('<location>/path/to/skill</location>');
    expect(result).toContain('</available_skills>');
  });

  it('should include instructions', () => {
    const result = buildSkillsSection([{ name: 's', description: 'd' }]);
    expect(result).toContain('mandatory');
    expect(result).toContain('scan <available_skills>');
  });
});

describe('buildToolsSection', () => {
  it('should return empty string for no tools', () => {
    expect(buildToolsSection([])).toBe('');
  });

  it('should list tools with descriptions', () => {
    const result = buildToolsSection([
      { name: 'memory_search', description: 'Search memory', parameters: {} },
      { name: 'web_search', description: 'Search the web', parameters: {} },
    ]);
    expect(result).toContain('memory_search');
    expect(result).toContain('Search memory');
    expect(result).toContain('web_search');
  });
});

describe('buildContextSection', () => {
  it('should include user name and timezone', () => {
    const result = buildContextSection({ displayName: 'Alice', timezone: 'UTC' });
    expect(result).toContain('Alice');
    expect(result).toContain('UTC');
  });

  it('should include memory context', () => {
    const result = buildContextSection({ memoryContext: 'User prefers Ethereum staking' });
    expect(result).toContain('User prefers Ethereum staking');
    expect(result).toContain('Project Context');
  });

  it('should return empty for no context', () => {
    const result = buildContextSection({});
    expect(result.trim()).toBe('');
  });
});
