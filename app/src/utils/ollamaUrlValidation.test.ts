import { describe, expect, it } from 'vitest';

import { validateOllamaUrl } from './ollamaUrlValidation';

describe('validateOllamaUrl', () => {
  it('accepts a plain http URL', () => {
    const result = validateOllamaUrl('http://localhost:11434');
    expect(result.valid).toBe(true);
    expect(result.normalized).toBe('http://localhost:11434');
  });

  it('accepts a plain https URL', () => {
    const result = validateOllamaUrl('https://remote-ollama.example.com:11434');
    expect(result.valid).toBe(true);
    expect(result.normalized).toBe('https://remote-ollama.example.com:11434');
  });

  it('accepts an IP address URL', () => {
    const result = validateOllamaUrl('http://192.168.1.5:11434');
    expect(result.valid).toBe(true);
    expect(result.normalized).toBe('http://192.168.1.5:11434');
  });

  it('rejects an empty string', () => {
    const result = validateOllamaUrl('');
    expect(result.valid).toBe(false);
    expect(result.error).toBeTruthy();
  });

  it('rejects a whitespace-only string', () => {
    const result = validateOllamaUrl('   ');
    expect(result.valid).toBe(false);
  });

  it('rejects URLs without http(s) scheme', () => {
    expect(validateOllamaUrl('localhost:11434').valid).toBe(false);
    expect(validateOllamaUrl('ftp://localhost:11434').valid).toBe(false);
  });

  it('rejects URLs with credentials', () => {
    const result = validateOllamaUrl('http://user:pass@localhost:11434');
    expect(result.valid).toBe(false);
    expect(result.error).toMatch(/credential/i);
  });

  it('strips path component and normalizes to scheme://host:port', () => {
    const result = validateOllamaUrl('http://192.168.1.5:11434/api/tags');
    expect(result.valid).toBe(true);
    expect(result.normalized).toBe('http://192.168.1.5:11434');
  });

  it('strips trailing slashes', () => {
    const result = validateOllamaUrl('http://localhost:11434///');
    expect(result.valid).toBe(true);
    expect(result.normalized).toBe('http://localhost:11434');
  });

  it('rejects URLs with query strings', () => {
    const result = validateOllamaUrl('http://localhost:11434?foo=bar');
    expect(result.valid).toBe(false);
  });

  it('rejects URLs with fragments', () => {
    const result = validateOllamaUrl('http://localhost:11434#section');
    expect(result.valid).toBe(false);
  });

  it('omits port from normalized URL when no port is specified', () => {
    const result = validateOllamaUrl('https://example.com');
    expect(result.valid).toBe(true);
    expect(result.normalized).toBe('https://example.com');
  });

  it('rejects a URL that starts with http:// but is not parseable', () => {
    const result = validateOllamaUrl('http:// has spaces');
    expect(result.valid).toBe(false);
    expect(result.error).toMatch(/invalid url format/i);
  });
});
