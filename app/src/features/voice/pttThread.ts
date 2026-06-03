/**
 * pttThread — thread-resolution adapter for pttService.
 *
 * Resolves which thread a PTT-captured message lands in:
 *   1. The currently-selected thread (state.thread.selectedThreadId) if any.
 *   2. Otherwise create a fresh thread via `threads_create_new`.
 *
 * Keeping this in its own module keeps `PttHotkeyManager` declarative — the
 * service interface only needs two thunks (`resolveActiveThreadId`,
 * `createNewVoiceThread`), and the redux access stays out of React render
 * scope.
 */
import { threadApi } from '../../services/api/threadApi';
import { store } from '../../store';

export async function resolveActiveThreadId(): Promise<string | null> {
  const state = store.getState();
  return state.thread.selectedThreadId ?? null;
}

export async function createNewVoiceThread(): Promise<string> {
  // No special "voice" label yet — the core auto-generates a title from the
  // first user message, which gives a useful label for PTT sessions too.
  const thread = await threadApi.createNewThread();
  return thread.id;
}
