import { describe, expect, it } from 'vitest';

import type { ConstitutionConfig } from '../../constitution/types';
import { buildSystemPrompt } from '../system-prompt';

const mockConstitution: ConstitutionConfig = {
  raw: '# Test Constitution',
  corePrinciples: [
    { id: 'principle-1', title: 'User sovereignty', description: 'The user owns their data.' },
  ],
  memoryPrinciples: [{ rule: 'Store facts and decisions' }],
  decisionFramework: [{ id: 'safety', question: 'Could this harm the user?' }],
  prohibitedActions: [{ description: 'Never execute trades without confirmation' }],
  interactionGuidelines: ['Use precise crypto terminology'],
  isDefault: true,
};

describe('buildSystemPrompt', () => {
  it('should include constitution section first', () => {
    const prompt = buildSystemPrompt({ constitution: mockConstitution });
    const constitutionPos = prompt.indexOf('Constitution');
    const identityPos = prompt.indexOf('Identity');
    expect(constitutionPos).toBeGreaterThan(-1);
    expect(identityPos).toBeGreaterThan(-1);
    expect(constitutionPos).toBeLessThan(identityPos);
  });

  it('should include core principles', () => {
    const prompt = buildSystemPrompt({ constitution: mockConstitution });
    expect(prompt).toContain('User sovereignty');
    expect(prompt).toContain('The user owns their data.');
  });

  it('should include prohibited actions', () => {
    const prompt = buildSystemPrompt({ constitution: mockConstitution });
    expect(prompt).toContain('Never execute trades without confirmation');
  });

  it('should include agent identity', () => {
    const prompt = buildSystemPrompt({
      constitution: mockConstitution,
      identity: { name: 'TestBot', tagline: 'A test assistant' },
    });
    expect(prompt).toContain('TestBot');
    expect(prompt).toContain('test assistant');
  });

  it('should default to AlphaHuman identity', () => {
    const prompt = buildSystemPrompt({ constitution: mockConstitution });
    expect(prompt).toContain('AlphaHuman');
  });

  it('should include crypto intelligence section', () => {
    const prompt = buildSystemPrompt({ constitution: mockConstitution });
    expect(prompt).toContain('Crypto Intelligence');
    expect(prompt).toContain('DeFi protocols');
    expect(prompt).toContain('On-chain analytics');
  });

  it('should include memory recall section in full mode', () => {
    const prompt = buildSystemPrompt({ constitution: mockConstitution, mode: 'full' });
    expect(prompt).toContain('Memory Recall');
    expect(prompt).toContain('memory_search');
  });

  it('should omit memory recall in minimal mode', () => {
    const prompt = buildSystemPrompt({ constitution: mockConstitution, mode: 'minimal' });
    expect(prompt).not.toContain('Memory Recall');
  });

  it('should include skills when provided in full mode', () => {
    const prompt = buildSystemPrompt({
      constitution: mockConstitution,
      mode: 'full',
      skills: [{ name: 'price-tracker', description: 'Track crypto prices' }],
    });
    expect(prompt).toContain('<available_skills>');
    expect(prompt).toContain('price-tracker');
    expect(prompt).toContain('Track crypto prices');
  });

  it('should omit skills in minimal mode', () => {
    const prompt = buildSystemPrompt({
      constitution: mockConstitution,
      mode: 'minimal',
      skills: [{ name: 'test', description: 'Test' }],
    });
    expect(prompt).not.toContain('<available_skills>');
  });

  it('should include tools when provided in full mode', () => {
    const prompt = buildSystemPrompt({
      constitution: mockConstitution,
      mode: 'full',
      tools: [{ name: 'memory_search', description: 'Search memory', parameters: {} }],
    });
    expect(prompt).toContain('Available Tools');
    expect(prompt).toContain('memory_search');
  });

  it('should include user context when provided', () => {
    const prompt = buildSystemPrompt({
      constitution: mockConstitution,
      userContext: { displayName: 'Alice', timezone: 'America/New_York' },
    });
    expect(prompt).toContain('Alice');
    expect(prompt).toContain('America/New_York');
  });

  it('should include crypto context when provided', () => {
    const prompt = buildSystemPrompt({
      constitution: mockConstitution,
      cryptoContext: {
        marketSummary: 'BTC at $95,000, ETH at $3,400',
        activeChains: ['Ethereum', 'Solana'],
      },
    });
    expect(prompt).toContain('BTC at $95,000');
    expect(prompt).toContain('Ethereum, Solana');
  });

  it('should return minimal prompt in none mode', () => {
    const prompt = buildSystemPrompt({ constitution: mockConstitution, mode: 'none' });
    expect(prompt).toContain('AlphaHuman');
    expect(prompt).not.toContain('Constitution');
    expect(prompt).not.toContain('Crypto Intelligence');
  });

  it('should use custom identity name in none mode', () => {
    const prompt = buildSystemPrompt({
      constitution: mockConstitution,
      identity: { name: 'CustomBot' },
      mode: 'none',
    });
    expect(prompt).toContain('CustomBot');
  });
});
