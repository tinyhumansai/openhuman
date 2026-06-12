import { defineConfig } from '@playwright/test';

const baseURL = process.env.PW_BASE_URL || 'http://127.0.0.1:4173';

export default defineConfig({
  testDir: './test/playwright/specs',
  fullyParallel: false,
  workers: 1,
  retries: process.env.CI ? 2 : 0,
  timeout: process.env.CI ? 90_000 : 60_000,
  expect: {
    timeout: process.env.CI ? 15_000 : 10_000,
  },
  use: {
    baseURL,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
  },
  reporter: [['list']],
});
