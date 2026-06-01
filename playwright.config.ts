import { defineConfig, devices } from "@playwright/test";

// E2E harness (#71). Runs against the static web build via `vite preview`.
// Note: Tauri invoke() calls have no backend in the browser, so these tests
// cover the UI shell (chrome, palette, navigation) — not backend flows.
// First run: `npx playwright install chromium`.
export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? "github" : "list",
  use: {
    baseURL: "http://localhost:4173",
    trace: "on-first-retry",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    command: "node_modules/.bin/vite preview --port 4173",
    url: "http://localhost:4173",
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
