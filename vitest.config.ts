import { defineConfig } from "vitest/config";

// Pruebas de los overlays (HTML/JS) ejecutados en jsdom. El sidecar tiene su propia configuración.
export default defineConfig({
  test: {
    include: ["overlays/test/**/*.test.ts", "scripts/**/*.test.mjs"],
    environment: "node",
    testTimeout: 10_000,
  },
});
