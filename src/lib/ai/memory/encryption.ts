import { invoke } from '@tauri-apps/api/core';

/**
 * Encryption layer that delegates to Rust Tauri commands.
 * All encryption operations use AES-256-GCM with Argon2id key derivation.
 */
export class MemoryEncryption {
  private password: string | null = null;
  private initialized = false;

  /**
   * Initialize encryption with a password.
   * Creates the key file if it doesn't exist.
   */
  async init(password: string): Promise<void> {
    const success = await invoke<boolean>('ai_init_encryption', { password });
    if (!success) {
      throw new Error('Failed to initialize encryption');
    }
    this.password = password;
    this.initialized = true;
  }

  /** Check if encryption has been initialized */
  isInitialized(): boolean {
    return this.initialized;
  }

  /**
   * Encrypt a string value.
   */
  async encrypt(plaintext: string): Promise<string> {
    if (!this.password) {
      throw new Error('Encryption not initialized');
    }
    return invoke<string>('ai_encrypt', { password: this.password, plaintext });
  }

  /**
   * Decrypt a string value.
   */
  async decrypt(encrypted: string): Promise<string> {
    if (!this.password) {
      throw new Error('Encryption not initialized');
    }
    return invoke<string>('ai_decrypt', { password: this.password, encrypted });
  }

  /**
   * Encrypt an embedding (Float32Array) to bytes for storage.
   */
  async encryptEmbedding(embedding: number[]): Promise<string> {
    const buffer = new Float32Array(embedding).buffer;
    const bytes = Array.from(new Uint8Array(buffer));
    const json = JSON.stringify(bytes);
    return this.encrypt(json);
  }

  /**
   * Decrypt stored embedding bytes back to number[].
   */
  async decryptEmbedding(encrypted: string): Promise<number[]> {
    const json = await this.decrypt(encrypted);
    const bytes: number[] = JSON.parse(json);
    const buffer = new ArrayBuffer(bytes.length);
    const view = new Uint8Array(buffer);
    for (let i = 0; i < bytes.length; i++) {
      view[i] = bytes[i];
    }
    return Array.from(new Float32Array(buffer));
  }

  /** Clear the stored password from memory */
  destroy(): void {
    this.password = null;
    this.initialized = false;
  }
}
