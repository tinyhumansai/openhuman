/**
 * Utilities for sanitizing sensitive data before logging
 */

const SENSITIVE_KEYS = [
  'token',
  'password',
  'apiKey',
  'api_key',
  'apikey',
  'secret',
  'auth',
  'authorization',
  'session',
  'sessionString',
  'session_string',
  'credentials',
  'privateKey',
  'private_key',
  'accessToken',
  'access_token',
  'refreshToken',
  'refresh_token',
];

const SENSITIVE_PATTERNS = [/password/i, /secret/i, /token/i, /key/i, /auth/i, /credential/i];

/**
 * Check if a key name suggests sensitive data
 */
function isSensitiveKey(key: string): boolean {
  const lowerKey = key.toLowerCase();
  return (
    SENSITIVE_KEYS.some(sk => lowerKey.includes(sk)) ||
    SENSITIVE_PATTERNS.some(pattern => pattern.test(key))
  );
}

/**
 * Sanitize an object by redacting sensitive values
 */
function sanitizeObject(obj: unknown, depth = 0): unknown {
  if (depth > 5) {
    return '[Max depth reached]';
  }

  if (obj === null || obj === undefined) {
    return obj;
  }

  if (typeof obj !== 'object') {
    return obj;
  }

  if (Array.isArray(obj)) {
    return obj.map(item => sanitizeObject(item, depth + 1));
  }

  const sanitized: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(obj)) {
    if (isSensitiveKey(key)) {
      sanitized[key] = '[REDACTED]';
    } else if (typeof value === 'object' && value !== null) {
      sanitized[key] = sanitizeObject(value, depth + 1);
    } else {
      sanitized[key] = value;
    }
  }

  return sanitized;
}

/**
 * Sanitize error objects, extracting only safe information
 */
export function sanitizeError(error: unknown): unknown {
  if (error instanceof Error) {
    const isDev = import.meta.env.DEV || import.meta.env.MODE === 'development';
    return { name: error.name, message: error.message, stack: isDev ? error.stack : undefined };
  }
  if (typeof error === 'object' && error !== null) {
    return sanitizeObject(error);
  }
  return error;
}

/**
 * Sanitize data for logging - removes sensitive fields and limits size
 */
export function sanitizeForLogging(data: unknown): unknown {
  if (data === null || data === undefined) {
    return data;
  }

  // For errors, use specialized sanitization
  if (data instanceof Error) {
    return sanitizeError(data);
  }

  // For objects, sanitize sensitive keys
  if (typeof data === 'object') {
    const sanitized = sanitizeObject(data);

    // If it's a large object, only show metadata
    const jsonStr = JSON.stringify(sanitized);
    if (jsonStr.length > 1000) {
      return {
        ...(typeof sanitized === 'object' && sanitized !== null
          ? { _truncated: true, _size: jsonStr.length }
          : {}),
        _preview: jsonStr.substring(0, 200) + '...',
      };
    }

    return sanitized;
  }

  return data;
}

/**
 * Create a safe log data object that only includes metadata
 */
export function createSafeLogData(
  metadata: Record<string, unknown>,
  sensitiveData?: unknown
): Record<string, unknown> {
  const safe: Record<string, unknown> = { ...metadata };

  if (sensitiveData !== undefined) {
    safe.hasData = true;
    safe.dataSize =
      typeof sensitiveData === 'string'
        ? sensitiveData.length
        : JSON.stringify(sensitiveData).length;

    // Only include sanitized preview for small objects
    const sanitized = sanitizeForLogging(sensitiveData);
    const jsonStr = JSON.stringify(sanitized);
    if (jsonStr.length <= 500) {
      safe.data = sanitized;
    } else {
      safe.dataPreview = jsonStr.substring(0, 200) + '...';
    }
  } else {
    safe.hasData = false;
  }

  return safe;
}
