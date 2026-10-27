// SPDX-License-Identifier: GPL-3.0-or-later
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  test: {
    environment: "node",
    exclude: ["**/node_modules/**", "e2e/**"],
  },
});
