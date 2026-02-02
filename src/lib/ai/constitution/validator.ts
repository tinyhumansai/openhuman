import type { ConstitutionConfig, ConstitutionValidation, ConstitutionViolation } from './types';

/** Patterns that indicate private key / seed phrase content */
const SECRET_PATTERNS = [
  /\b(0x)?[0-9a-fA-F]{64}\b/, // Private key hex
  /\b(\w+\s+){11,23}\w+\b/, // Mnemonic seed phrase (12/24 words)
  /\bseed\s*phrase\b/i,
  /\bprivate\s*key\b/i,
  /\bsecret\s*key\b/i,
  /\bkeystore\b/i,
];

/** Patterns that look like financial advice */
const FINANCIAL_ADVICE_PATTERNS = [
  /\byou\s+should\s+buy\b/i,
  /\byou\s+should\s+sell\b/i,
  /\bguaranteed\s+returns?\b/i,
  /\brisk[\s-]free\b/i,
  /\bcan't?\s+lose\b/i,
];

/**
 * Validate content against constitutional rules.
 * Used before writing to memory or sending responses.
 */
export function validateMemoryContent(
  content: string,
  _constitution: ConstitutionConfig
): ConstitutionValidation {
  const violations: ConstitutionViolation[] = [];

  // Check for secrets/credentials
  for (const pattern of SECRET_PATTERNS) {
    if (pattern.test(content)) {
      violations.push({
        rule: 'Never persist private keys, seed phrases, or raw credentials in memory',
        category: 'privacy',
        severity: 'error',
        message: `Content appears to contain sensitive cryptographic material matching pattern: ${pattern.source}`,
      });
    }
  }

  return { valid: violations.length === 0, violations };
}

/**
 * Validate an action/recommendation against constitutional rules.
 */
export function validateAction(
  actionDescription: string,
  _constitution: ConstitutionConfig
): ConstitutionValidation {
  const violations: ConstitutionViolation[] = [];

  // Check for financial advice patterns
  for (const pattern of FINANCIAL_ADVICE_PATTERNS) {
    if (pattern.test(actionDescription)) {
      violations.push({
        rule: 'No financial advice — Provide information and analysis, not directives',
        category: 'accuracy',
        severity: 'warning',
        message: 'Content may constitute financial advice. Ensure DYOR disclaimer is included.',
      });
    }
  }

  return { valid: violations.length === 0, violations };
}

/**
 * Sanitize content by redacting detected secrets before storage.
 */
export function sanitizeForMemory(content: string): string {
  let sanitized = content;

  // Redact potential private keys (64 hex chars)
  sanitized = sanitized.replace(/\b(0x)?[0-9a-fA-F]{64}\b/g, '[REDACTED_KEY]');

  // Redact potential seed phrases (12+ words that look like a mnemonic)
  // Only redact if the words match common BIP39 word patterns
  sanitized = sanitized.replace(
    /\b(abandon|ability|able|about|above|absent|absorb|abstract|absurd|abuse)\s+\w+(\s+\w+){10,22}\b/gi,
    '[REDACTED_SEED_PHRASE]'
  );

  return sanitized;
}
