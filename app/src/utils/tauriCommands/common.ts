/**
 * Common utilities and types for Tauri Commands.
 */
import { isTauri as coreIsTauri } from '@tauri-apps/api/core';

// Check if we're running in Tauri
export const isTauri = (): boolean => {
  // Tauri v2: prefer the official runtime check over window globals.
  return coreIsTauri();
};

export interface CommandResponse<T> {
  result: T;
  logs: string[];
}

export function tauriErrorMessage(err: unknown): string {
  if (err instanceof Error && err.message) {
    return err.message;
  }
  if (typeof err === 'string') {
    return err;
  }
  if (err && typeof err === 'object') {
    const maybeMessage = (err as { message?: unknown }).message;
    if (typeof maybeMessage === 'string' && maybeMessage.trim().length > 0) {
      return maybeMessage;
    }
    const maybeError = (err as { error?: unknown }).error;
    if (typeof maybeError === 'string' && maybeError.trim().length > 0) {
      return maybeError;
    }
  }
  return 'Unknown Tauri invoke error';
}

export function parseServiceCliOutput<T>(raw: string): CommandResponse<T> {
  const parsed = JSON.parse(raw) as CommandResponse<T>;
  return parsed;
}
