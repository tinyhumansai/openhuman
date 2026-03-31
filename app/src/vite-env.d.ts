/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_OPENHUMAN_CORE_RPC_URL?: string;
  readonly VITE_BACKEND_URL?: string;
  readonly VITE_SKILLS_GITHUB_REPO?: string;
  readonly VITE_SENTRY_DSN?: string;
  readonly VITE_DEV_JWT_TOKEN?: string;
  readonly VITE_DEV_FORCE_ONBOARDING?: string;
  readonly DEV: boolean;
  readonly MODE: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}

// Node.js polyfills for browser
declare global {
  interface Window {
    Buffer: typeof Buffer;
    process: typeof process;
    util: typeof import('util');
  }
  var Buffer: typeof import('buffer').Buffer;
  var process: typeof import('process');
  var util: typeof import('util');
}
