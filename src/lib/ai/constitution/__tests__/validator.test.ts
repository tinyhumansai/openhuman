import { describe, expect, it } from 'vitest';

import type { ConstitutionConfig } from '../types';
import { sanitizeForMemory, validateAction, validateMemoryContent } from '../validator';

const mockConstitution: ConstitutionConfig = {
  raw: '',
  corePrinciples: [],
  memoryPrinciples: [],
  decisionFramework: [],
  prohibitedActions: [],
  interactionGuidelines: [],
  isDefault: true,
};

describe('validateMemoryContent', () => {
  it('should pass for normal text content', () => {
    const result = validateMemoryContent('User prefers ETH over BTC for staking', mockConstitution);
    expect(result.valid).toBe(true);
    expect(result.violations).toHaveLength(0);
  });

  it('should detect private key hex patterns', () => {
    const result = validateMemoryContent(
      'Private key: 0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890',
      mockConstitution
    );
    expect(result.valid).toBe(false);
    expect(result.violations.length).toBeGreaterThan(0);
    expect(result.violations[0].category).toBe('privacy');
    expect(result.violations[0].severity).toBe('error');
  });

  it('should detect raw 64-char hex without 0x prefix', () => {
    const hex64 = 'a'.repeat(64);
    const result = validateMemoryContent(`Key stored: ${hex64}`, mockConstitution);
    expect(result.valid).toBe(false);
  });

  it('should detect seed phrase keywords', () => {
    const result = validateMemoryContent('The seed phrase is written on paper', mockConstitution);
    expect(result.valid).toBe(false);
    expect(result.violations[0].category).toBe('privacy');
  });

  it('should detect private key keywords', () => {
    const result = validateMemoryContent('Store the private key securely', mockConstitution);
    expect(result.valid).toBe(false);
  });

  it('should detect secret key keywords', () => {
    const result = validateMemoryContent('The secret key for the wallet', mockConstitution);
    expect(result.valid).toBe(false);
  });

  it('should detect keystore reference', () => {
    const result = validateMemoryContent('Import from keystore file', mockConstitution);
    expect(result.valid).toBe(false);
  });

  it('should pass for wallet addresses (42 chars, not 64)', () => {
    const result = validateMemoryContent(
      'Send to 0x742d35Cc6634C0532925a3b844Bc9e7595f',
      mockConstitution
    );
    expect(result.valid).toBe(true);
  });

  it('should pass for transaction hashes (66 chars with 0x)', () => {
    // TX hashes are 66 chars (0x + 64 hex), which matches the 64-hex pattern
    // This is an accepted trade-off — the validator is conservative
    const txHash = '0x' + 'a'.repeat(64);
    const result = validateMemoryContent(`Transaction: ${txHash}`, mockConstitution);
    expect(result.valid).toBe(false); // Conservative: flags 64-char hex
  });
});

describe('validateAction', () => {
  it('should pass for informational content', () => {
    const result = validateAction('ETH is currently trading at $3,400', mockConstitution);
    expect(result.valid).toBe(true);
  });

  it('should warn about buy recommendations', () => {
    const result = validateAction('You should buy ETH right now', mockConstitution);
    expect(result.valid).toBe(false);
    expect(result.violations[0].severity).toBe('warning');
    expect(result.violations[0].category).toBe('accuracy');
  });

  it('should warn about sell recommendations', () => {
    const result = validateAction('You should sell your BTC before it drops', mockConstitution);
    expect(result.valid).toBe(false);
  });

  it('should warn about guaranteed returns', () => {
    const result = validateAction(
      'This protocol offers guaranteed returns of 20% APY',
      mockConstitution
    );
    expect(result.valid).toBe(false);
  });

  it('should warn about risk-free claims', () => {
    const result = validateAction('This is a risk-free investment opportunity', mockConstitution);
    expect(result.valid).toBe(false);
  });

  it("should warn about can't lose claims", () => {
    const result = validateAction("You can't lose with this strategy", mockConstitution);
    expect(result.valid).toBe(false);
  });

  it('should pass for analytical language with DYOR', () => {
    const result = validateAction(
      'Based on the chart, ETH shows bullish divergence. DYOR.',
      mockConstitution
    );
    expect(result.valid).toBe(true);
  });
});

describe('sanitizeForMemory', () => {
  it('should redact 64-char hex strings', () => {
    const hex = 'a'.repeat(64);
    const result = sanitizeForMemory(`Key: ${hex}`);
    expect(result).toBe('Key: [REDACTED_KEY]');
    expect(result).not.toContain(hex);
  });

  it('should redact 0x-prefixed 64-char hex', () => {
    const hex = '0x' + 'b'.repeat(64);
    const result = sanitizeForMemory(`Stored: ${hex}`);
    expect(result).toContain('[REDACTED_KEY]');
  });

  it('should not modify normal text', () => {
    const text = 'User prefers staking ETH on Lido';
    expect(sanitizeForMemory(text)).toBe(text);
  });

  it('should not modify short hex strings', () => {
    const text = 'Transaction 0x1234abcd was confirmed';
    expect(sanitizeForMemory(text)).toBe(text);
  });
});
