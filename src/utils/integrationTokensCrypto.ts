/**
 * Helpers for encrypting/decrypting OAuth integration tokens.
 * Matches backend format: IV (16 bytes) + AuthTag (16 bytes) + EncryptedData.
 * IV is derived from SHA-256(message) for encryption (deterministic).
 */

export type IntegrationTokensPayload = {
  accessToken: string;
  refreshToken: string;
  /** ISO timestamp string */
  expiresAt: string;
};

/** Stored value: encrypted blob only; decrypt with key when needed */
export type StoredIntegrationTokens = { encrypted: string };

const HEX_REGEX = /^[0-9a-fA-F]*$/;

export function hexToBytes(hex: string): Uint8Array {
  const cleanHex = hex.trim().replace(/^0x/i, '');
  if (!cleanHex) return new Uint8Array();
  if (cleanHex.length % 2 !== 0) {
    throw new TypeError(
      `hexToBytes: hex string must have even length (got ${cleanHex.length})`
    );
  }
  if (!HEX_REGEX.test(cleanHex)) {
    throw new TypeError(
      'hexToBytes: hex string must contain only [0-9a-fA-F] characters'
    );
  }
  const bytes = new Uint8Array(cleanHex.length / 2);
  for (let i = 0; i < cleanHex.length; i += 2) {
    bytes[i / 2] = parseInt(cleanHex.slice(i, i + 2), 16);
  }
  return bytes;
}

export function hexToBase64(hex: string): string {
  const bytes = hexToBytes(hex);
  if (bytes.length === 0) return '';
  let binary = '';
  for (let i = 0; i < bytes.length; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary);
}

export function base64ToBytes(b64: string): Uint8Array {
  let normalized = b64.replace(/-/g, '+').replace(/_/g, '/');
  const pad = normalized.length % 4;
  if (pad === 2) normalized += '==';
  else if (pad === 3) normalized += '=';

  const binary = atob(normalized);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

/**
 * Decrypt an encrypted tokens payload (base64) using a 32-byte key (hex).
 * Backend format: IV (16) + AuthTag (16) + EncryptedData.
 */
export async function decryptIntegrationTokens(
  encryptedPayload: string,
  keyHex: string
): Promise<string> {
  if (typeof crypto === 'undefined' || !crypto.subtle) {
    throw new Error('Web Crypto API is not available for decryption');
  }

  const keyBytes = hexToBytes(keyHex);
  if (keyBytes.length !== 32) {
    throw new Error('Invalid encryption key: expected 32-byte AES-GCM key');
  }

  const combined = base64ToBytes(encryptedPayload);
  if (combined.length <= 32) {
    throw new Error('Encrypted payload too short');
  }

  const iv = combined.slice(0, 16);
  const authTag = combined.slice(16, 32);
  const encryptedData = combined.slice(32);
  const ciphertextWithTag = new Uint8Array(encryptedData.length + authTag.length);
  ciphertextWithTag.set(encryptedData, 0);
  ciphertextWithTag.set(authTag, encryptedData.length);

  const cryptoKey = await crypto.subtle.importKey(
    'raw',
    keyBytes as unknown as BufferSource,
    { name: 'AES-GCM' },
    false,
    ['decrypt']
  );

  const decrypted = await crypto.subtle.decrypt(
    { name: 'AES-GCM', iv, tagLength: 128 },
    cryptoKey,
    ciphertextWithTag as unknown as BufferSource
  );

  return new TextDecoder().decode(decrypted);
}

/**
 * Encrypt a plaintext string (e.g. JSON) using a 32-byte key (hex).
 * Matches backend: deterministic IV = first 16 bytes of SHA-256(message).
 * Returns base64(IV + AuthTag + EncryptedData).
 */
export async function encryptIntegrationTokens(plaintext: string, keyHex: string): Promise<string> {
  if (typeof crypto === 'undefined' || !crypto.subtle) {
    throw new Error('Web Crypto API is not available for encryption');
  }

  const keyBytes = hexToBytes(keyHex);
  if (keyBytes.length !== 32) {
    throw new Error('Invalid encryption key: expected 32-byte AES-GCM key');
  }

  try {
    const payload = JSON.parse(plaintext) as Record<string, unknown>;
    if (typeof payload.expiresAt !== 'string' || !payload.expiresAt.trim()) {
      throw new Error(
        'Payload must include a non-empty expiresAt field when using deterministic IV'
      );
    }
  } catch (e) {
    if (e instanceof SyntaxError) {
      throw new Error(
        'Plaintext must be JSON with a non-empty expiresAt field when using deterministic IV'
      );
    }
    throw e;
  }

  const messageBytes = new TextEncoder().encode(plaintext);

  // Deterministic IV for backend compatibility. TODO: Prefer a random 12-byte IV
  // (crypto.getRandomValues) prepended to ciphertext; update decrypt to handle both formats.
  const hashBuffer = await crypto.subtle.digest('SHA-256', messageBytes);
  const iv = new Uint8Array(hashBuffer).slice(0, 16);

  const cryptoKey = await crypto.subtle.importKey(
    'raw',
    keyBytes as unknown as BufferSource,
    { name: 'AES-GCM' },
    false,
    ['encrypt']
  );

  const encryptedBuffer = await crypto.subtle.encrypt(
    { name: 'AES-GCM', iv, tagLength: 128 },
    cryptoKey,
    messageBytes
  );

  const encryptedArray = new Uint8Array(encryptedBuffer);
  const authTag = encryptedArray.slice(-16);
  const encryptedData = encryptedArray.slice(0, -16);

  const combined = new Uint8Array(iv.length + authTag.length + encryptedData.length);
  combined.set(iv, 0);
  combined.set(authTag, iv.length);
  combined.set(encryptedData, iv.length + authTag.length);

  let binary = '';
  for (let i = 0; i < combined.length; i++) {
    binary += String.fromCharCode(combined[i]);
  }
  return btoa(binary);
}
