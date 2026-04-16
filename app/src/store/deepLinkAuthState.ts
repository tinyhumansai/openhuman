import { useSyncExternalStore } from 'react';

export interface DeepLinkAuthState {
  isProcessing: boolean;
  errorMessage: string | null;
}

const initialState: DeepLinkAuthState = { isProcessing: false, errorMessage: null };

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
  setDeepLinkAuthState({ isProcessing: true, errorMessage: null });
};

export const completeDeepLinkAuthProcessing = (): void => {
  setDeepLinkAuthState({ isProcessing: false, errorMessage: null });
};

export const failDeepLinkAuthProcessing = (message: string): void => {
  setDeepLinkAuthState({ isProcessing: false, errorMessage: message });
};

export const useDeepLinkAuthState = (): DeepLinkAuthState =>
  useSyncExternalStore(subscribeDeepLinkAuthState, getDeepLinkAuthState, getDeepLinkAuthState);
