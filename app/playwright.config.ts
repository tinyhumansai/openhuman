import { defineConfig } from '@playwright/test';

const baseURL = process.env.PW_BASE_URL || 'http://127.0.0.1:4173';

export default defineConfig({
  testDir: './test/playwright/specs',
  fullyParallel: false,
  workers: 1,
  timeout: 60_000,
  expect: {
    timeout: 10_000,
  },
  use: {
    baseURL,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
  },
  reporter: [['list']],
});
