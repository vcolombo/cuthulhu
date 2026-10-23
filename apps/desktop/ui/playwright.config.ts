// SPDX-License-Identifier: GPL-3.0-or-later
import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "e2e",
  // The flag is baked when Vite starts, so the menu-on suite cannot share the menu-off server.
  webServer: [
    {
      command: "npm run dev",
      port: 5173,
      reuseExistingServer: !process.env.CI,
    },
    {
      command: "npm run dev -- --port 5174 --strictPort",
      // Merged over process.env. A Unix `VAR=value` prefix fails on Windows cmd, and that
      // failure stops every webServer, including the suite that does not use the flag.
      env: { VITE_CUTHULHU_DEMO_STATES: "1" },
      port: 5174,
      reuseExistingServer: !process.env.CI,
    },
  ],
  use: {
    baseURL: "http://localhost:5173",
  },
  projects: [
    { name: "chromium", testIgnore: /demo\.spec\.ts/, use: { ...devices["Desktop Chrome"] } },
    {
      name: "demo",
      testMatch: /demo\.spec\.ts/,
      use: { ...devices["Desktop Chrome"], baseURL: "http://localhost:5174" },
    },
  ],
});
