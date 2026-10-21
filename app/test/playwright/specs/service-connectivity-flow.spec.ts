import { test } from '@playwright/test';

// Service availability prompts depend on the native Tauri service mock and are
// covered by the desktop runner, not the browser-hosted Playwright lane.
