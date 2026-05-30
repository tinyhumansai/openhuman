export interface OllamaUrlValidationResult {
  valid: boolean;
  normalized?: string;
  error?: string;
}

/**
 * Validate and normalize a user-supplied Ollama base URL.
 *
 * Rules (mirrors the Rust `validate_ollama_url` helper):
 * - Trims whitespace and strips trailing slashes
 * - Must be http:// or https://
 * - Must have a non-empty hostname
 * - No credentials (user:pass@)
 * - No query string or fragment
 * - Path component is stripped — normalized form is scheme://host[:port]
 */
export function validateOllamaUrl(raw: string): OllamaUrlValidationResult {
  const trimmed = raw.trim().replace(/\/+$/, '');
  if (!trimmed) {
    return { valid: false, error: 'URL must not be empty' };
  }
  if (!trimmed.startsWith('http://') && !trimmed.startsWith('https://')) {
    return { valid: false, error: 'Must be a valid http:// or https:// URL' };
  }
  let parsed: URL;
  try {
    parsed = new URL(trimmed);
  } catch {
    return { valid: false, error: 'Invalid URL format' };
  }
  if (!parsed.hostname) {
    return { valid: false, error: 'URL must have a non-empty host' };
  }
  if (parsed.username || parsed.password) {
    return { valid: false, error: 'URL must not contain credentials (user:pass@host)' };
  }
  if (parsed.search) {
    return { valid: false, error: 'URL must not contain a query string' };
  }
  if (parsed.hash) {
    return { valid: false, error: 'URL must not contain a fragment' };
  }
  // Normalize to scheme://host[:port]
  const port = parsed.port ? `:${parsed.port}` : '';
  const normalized = `${parsed.protocol}//${parsed.hostname}${port}`;
  return { valid: true, normalized };
}
