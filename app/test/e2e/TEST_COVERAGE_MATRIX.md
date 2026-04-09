# Backend Flow Coverage Matrix (Updated)

This matrix aligns the E2E/backend-flow suite with the 0-11 flow list.

## Implemented specs (current app flows)

- `1 Authentication`: `specs/login-flow.spec.ts`, `specs/logout-relogin-onboarding.spec.ts`, `specs/auth-session-management.spec.ts`
- `3 Local AI Runtime`: `specs/local-model-runtime.spec.ts`
- `7 Chat Interface`: `specs/chat-interface-flow.spec.ts`
- `8 Integrations (Channels)`: `specs/telegram-flow.spec.ts`, `specs/discord-flow.spec.ts`
- `8 Integrations (3rd Party Skills)`: `specs/gmail-flow.spec.ts`, `specs/notion-flow.spec.ts`, `specs/skill-oauth.spec.ts`
- `9 Built-in Skills`: `specs/screen-intelligence.spec.ts`, `specs/voice-mode.spec.ts`, `specs/text-autocomplete-flow.spec.ts`
- `System health/navigation`: `specs/service-connectivity-flow.spec.ts`, `specs/skills-registry.spec.ts`, `specs/navigation.spec.ts`, `specs/smoke.spec.ts`, `specs/tauri-commands.spec.ts`

## Added missing flow specs (now executable)

- `0 macOS distribution`: `specs/macos-distribution.spec.ts` (macOS-only checks; skipped off macOS)
- `1 auth extensions`: `specs/auth-session-management.spec.ts`
- `2 + 4 permissions/system tools`: `specs/permissions-system-access.spec.ts`
- `5 memory`: `specs/memory-system.spec.ts`
- `6 automation/scheduling`: `specs/automation-scheduling.spec.ts`
- `8 + 9 integrations & skills`: `specs/chat-skills-integrations.spec.ts`
- `10 + 11 rewards/settings`: `specs/rewards-settings.spec.ts`

## Removed obsolete specs

- `specs/card-payment-flow.spec.ts`
- `specs/crypto-payment-flow.spec.ts`
- `specs/auth-access-control.spec.ts`

These files mapped to older payment/subscription-era flows and no longer align with the latest backend test matrix.
