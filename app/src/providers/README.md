# App Providers

This directory contains the React context providers that manage the global state and services of the application.

## CoreStateProvider

Manages the authoritative global state of the application, including user authentication, session tokens, and the application snapshot.

### Turn-Boundary Refetch Contract

To ensure that the UI stays in sync with the backend state (especially during onboarding and context gathering), the application follows a refetch-on-turn-end contract:

- **Refetch Timing**: After every agent reply completes (the `chat_done` event in `ChatRuntimeProvider`), the application refetches the authoritative user state via `userApi.getMe()`.
- **Debounce**: Multiple rapid turn-finalized events within 750ms are collapsed into a single refetch call to avoid unnecessary network traffic.
- **Single Source of Truth**: The refetched state is merged into the global snapshot using `patchSnapshot`. Components should bind to this global snapshot to ensure they reflect the latest backend state without requiring a full remount.
- **Fire-and-Forget**: The refetch operation is non-blocking and fires on a microtask after the chat UI has painted the final response.

## ChatRuntimeProvider

Manages the live chat state, including message streaming, tool execution timeline, and subagent orchestration. It subscribes to socket events and updates the Redux store.

## SocketProvider

Manages the Socket.IO connection to the Rust core, providing the underlying transport for real-time chat events.
