export const BACKEND_URL = import.meta.env.VITE_BACKEND_URL || 'https://api.alphahuman.xyz';

export const TELEGRAM_BOT_USERNAME =
  import.meta.env.VITE_TELEGRAM_BOT_USERNAME || 'alphahumanx_bot';

export const TELEGRAM_BOT_ID = import.meta.env.VITE_TELEGRAM_BOT_ID || '8043922470';

export const TELEGRAM_API_ID = import.meta.env.VITE_TELEGRAM_API_ID
  ? Number(import.meta.env.VITE_TELEGRAM_API_ID)
  : undefined;

export const TELEGRAM_API_HASH = import.meta.env.VITE_TELEGRAM_API_HASH || undefined;

export const IS_DEV = Boolean(import.meta.env.DEV) || import.meta.env.MODE === 'development';

export const SKILLS_GITHUB_REPO = import.meta.env.VITE_SKILLS_GITHUB_REPO || 'alphahumanxyz/skills';

export const SKILLS_GITHUB_TOKEN = import.meta.env.VITE_SKILLS_GITHUB_TOKEN || undefined;
