import { useSyncExternalStore } from 'react';

interface DeepLinkAuthState {
  isProcessing: boolean;
  errorMessage: string | null;
  // i18n key to render INSTEAD of `errorMessage`, for failures whose copy is
  // translated. This module is reached from non-React code (the deep-link
  // listener), so it cannot call `useT()` itself; carrying the key lets the
  // rendering component resolve it in the user's locale. Null keeps the
  // existing behaviour of rendering `errorMessage` verbatim.
  errorMessageKey: string | null;
  // Set when sign-in fails because the local core could not decrypt persisted
  // secrets — typically the encryption key on disk no longer matches the
  // ciphertext (key rotated, profile copied between machines, tampered/corrupt
  // storage). The only safe recovery is wiping local app data so the next
  // login starts from a clean slate.
  requiresAppDataReset: boolean;
}

const initialState: DeepLinkAuthState = {
  isProcessing: false,
  errorMessage: null,
  errorMessageKey: null,
  requiresAppDataReset: false,
};

let deepLinkAuthState: DeepLinkAuthState = initialState;
const listeners = new Set<() => void>();

const emitChange = (): void => {
  for (const listener of listeners) {
    listener();
  }
};

const setDeepLinkAuthState = (next: DeepLinkAuthState): void => {
  deepLinkAuthState = next;
  emitChange();
};

export const getDeepLinkAuthState = (): DeepLinkAuthState => deepLinkAuthState;

export const subscribeDeepLinkAuthState = (listener: () => void): (() => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};

export const beginDeepLinkAuthProcessing = (): void => {
  setDeepLinkAuthState({
    isProcessing: true,
    errorMessage: null,
    errorMessageKey: null,
    requiresAppDataReset: false,
  });
};

export const completeDeepLinkAuthProcessing = (): void => {
  setDeepLinkAuthState({
    isProcessing: false,
    errorMessage: null,
    errorMessageKey: null,
    requiresAppDataReset: false,
  });
};

export const failDeepLinkAuthProcessing = (
  message: string,
  options: { requiresAppDataReset?: boolean; messageKey?: string } = {}
): void => {
  setDeepLinkAuthState({
    isProcessing: false,
    errorMessage: message,
    errorMessageKey: options.messageKey ?? null,
    requiresAppDataReset: Boolean(options.requiresAppDataReset),
  });
};

export const useDeepLinkAuthState = (): DeepLinkAuthState =>
  useSyncExternalStore(subscribeDeepLinkAuthState, getDeepLinkAuthState, getDeepLinkAuthState);
