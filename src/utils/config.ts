export const BACKEND_URL = import.meta.env.VITE_BACKEND_URL || 'https://api.tinyhumans.ai';

export const TELEGRAM_BOT_USERNAME =
  import.meta.env.VITE_TELEGRAM_BOT_USERNAME || 'alphahumanx_bot';

export const TELEGRAM_BOT_ID = import.meta.env.VITE_TELEGRAM_BOT_ID || '8043922470';

export const IS_DEV = import.meta.env.DEV;

export const SKILLS_GITHUB_REPO = import.meta.env.VITE_SKILLS_GITHUB_REPO || 'alphahumanxyz/skills';

export const DEV_AUTO_LOAD_SKILL = import.meta.env.VITE_DEV_AUTO_LOAD_SKILL || undefined;
