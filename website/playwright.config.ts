import { defineConfig, devices } from '@playwright/test';
export default defineConfig({
  testDir: './tests',
  use: { baseURL: process.env.DOCS_URL || 'http://127.0.0.1:4321', trace: 'retain-on-failure' },
  webServer: process.env.DOCS_URL ? undefined : {
    command: 'python3 scripts/serve-preview.py',
    url: 'http://127.0.0.1:4321/GraphFusion/',
    reuseExistingServer: !process.env.CI,
  },
  projects: [
    { name: 'desktop', use: { ...devices['Desktop Chrome'], viewport: { width: 1440, height: 1000 } } },
    { name: 'mobile', use: { ...devices['iPhone 13'], defaultBrowserType: 'chromium' } },
  ],
});
